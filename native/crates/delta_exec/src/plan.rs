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
//!   A line that wants a wider search, where its work pays `WIDE_PAYS` times its cost, has it inline (0203 §2, §5):
//!   exact up to `WIDE_GREEDY` steps and `RANDOM_ORDERS` random connected orders, the best `PROPOSALS` its
//!   proposals; any other line's proposals are priced again beside its runners-up.
//! - **choose** (a map): the cheapest candidate, its price corrected by what its order measured, a geometric mean
//!   over runs (0203 §6); now and then (`EXPLORE_RATE`) an untried runner-up within `MARGIN` of it, for a line that
//!   matters (on the critical path with its share, or slow: §7) priced under `EXPLORE_ROWS`, by a seeded draw from
//!   the line's key and its runs (a pure function of them). At a handoff it takes the order a trial measured least,
//!   where it beats the running one by `TRIAL_MARGIN` (0207), or a slow line's search from its cut, untrialled.
//! - **emit_plan** (an emit): the chosen chains into the plan block `seed`, `step` and `head` read (each step's
//!   arguments bound again in the new order), and each priced line's choice and runners-up into the memo.
//! - **check** (a map, after each depth): each line's work so far against its limit, at its due points (`SLOW_ROWS`,
//!   then each doubling): past `SLOW_FACTOR` times its price, or a step missed (0203 §3a), it is slow (§3); else a
//!   line on the critical path whose orders are untrialled is due a trial (0207 §2, 0274).
//! - **stats** (a map, for slow lines): the exact rows of each pair of the line's atoms that share a variable, into
//!   the memo (0203 §3's exact pair statistics), which the next pricing uses.
//! - **note** (a map, at the round's end): what each line measured, into the memo: its work and its first step's
//!   rows (the budget's next test), its order's measured ratio, and each step's sample (its rows in and out against
//!   its price: the learned model's training rows, `samples`). A slow line asks for a wider search.
//! - **weigh**, **critical**, **mark** (at a level's run's end, `PlanOps::critical_path`): each stratum's work this
//!   run, the longest chain of producers by it, and each line's share of that path into its note, a moving average;
//!   a critical line (`CRITICAL` of it, `CRITICAL_ROWS` of work) asks for a wider search while its order measured
//!   over `CRITICAL_RATIO` times its price or it has no proposals (0203 §4).
//!
//! A slow line is planned again from the depth it reached (0206's cut): `stats`, `price` and `search` with the steps
//! it ran as a fixed prefix and its tuples there as the rows in. Where it is on the critical path (0274), its
//! candidates are trialled over a sample of those tuples (0207: `TRIAL_ROWS`, each cut at `TRIAL_CUTOFF` times the
//! least measured so far, the sample what `TRIAL_SHARE` of its work pays for), as are those of a line due a trial;
//! off the path, a slow line goes on in its search's order. The one that goes on is given a limit (0271,
//! `limit_again`). Its steps
//! already run keep their places, so the tuples made so far read the same in the new order.
//!
//! The memo is the stratum's operator cache (P§11.3, L5): dropping it changes what is planned, never an answer, and
//! every order a planner emits is a valid order of its chain's steps (a filter after its variables are bound).

use crate::body::{self, Arg, BodyOps, Env, Op, Plan, Side, StepPlan, TermPlan};
use crate::dir::{BlockId, Directory};
use crate::op::{Across, Bound, Found, Independence, Inside, Mode, OpId, Output, Over, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use crate::program::Cmp;
use crate::shapes::Facts;
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

/// Orders tried a line keeps, and a run's weight in an order's measured ratio, a geometric mean over runs (0203 §6).
pub const TRIED: usize = 8;
pub const RUN_WEIGHT: f64 = 0.5;
/// A line on the run's critical path with this share of the path's work, past `CRITICAL_ROWS` of its own, is critical
/// (0203 §4): a wider search is asked for it while its order measured over `CRITICAL_RATIO` times its price or it has
/// no proposals.
pub const CRITICAL: f64 = 0.05;
pub const CRITICAL_ROWS: f64 = 10_000.0;
pub const CRITICAL_RATIO: f64 = 2.0;
/// The wider search (0203 §5): exact up to `WIDE_GREEDY` steps (a round's is `GREEDY`) with its runners-up, and
/// `RANDOM_ORDERS` random connected orders, each priced as a kept order is; the best `PROPOSALS`, distinct, are the
/// line's proposals. It is made inline at the line's next plan where its measured work is `WIDE_PAYS` times the
/// search's cost (0203 §2; 0122: planning never costs more than running).
pub const WIDE_GREEDY: usize = 16;
pub const RANDOM_ORDERS: usize = 48;
pub const PROPOSALS: usize = 6;
pub const WIDE_PAYS: f64 = 100.0;
/// A step miss (0203 §3a), judged once a line has done `SLOW_FACTOR` searches' work: a step over-estimated is one
/// entered with at least `MISS_BINDINGS` bindings.
pub const MISS_BINDINGS: f64 = 64.0;

/// A search's states over `free` steps under a bound of `greedy`: the subsets and their last steps, or past the bound
/// the greedy order's candidates.
fn states(free: usize, greedy: usize) -> f64 {
    if free <= greedy { (1u64 << free) as f64 * free as f64 } else { (free * free) as f64 }
}

/// A wider search's cost in rows of join work: a round's search is `SEARCH_WORK` (0122), the exact part that times its
/// states over a round's, and a row for each step of each random order priced.
pub fn wide_cost(free: usize) -> f64 {
    SEARCH_WORK * states(free, WIDE_GREEDY) / states(free, GREEDY).max(1.0) + (RANDOM_ORDERS * free) as f64
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

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
    /// Wider searches made inline (0203 §5), and lines the run's critical path marked critical (§4).
    pub wide: u64,
    pub critical: u64,
    /// Lines given their default plan by their relations' declarations (0361), and those whose rows broke them.
    pub declared: u64,
    pub broken: u64,
    /// Recursions run along a declared tree in one wave, unplanned (0361's algorithms; Operations, class 30).
    pub waves: u64,
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
        self.wide += o.wide;
        self.critical += o.critical;
        self.declared += o.declared;
        self.broken += o.broken;
        self.waves += o.waves;
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
                Op::Atom | Op::AggKeys => n.binds = vars,
                // An aggregate: its own variables (found nowhere else) and its answer it binds; its keys it needs.
                Op::Agg(..) => {
                    let own = (0..occ.len()).filter(|&x| vars >> x & 1 == 1 && occ[x] == 1 << j && in_out >> x & 1 == 0).fold(0u64, |m, x| m | 1 << x);
                    let answer = if let Some(V::Var(x)) = n.args.last() { 1u64 << x } else { 0 };
                    n.binds = own | answer;
                    n.needs = vars & !own & !answer;
                }
                Op::Not => {
                    let own = (0..occ.len()).filter(|&x| vars >> x & 1 == 1 && occ[x] == 1 << j && in_out >> x & 1 == 0).fold(0u64, |m, x| m | 1 << x);
                    n.binds = own;
                    n.needs = vars & !own;
                }
                Op::Cmp(_) => n.needs = vars,
                // An operator needs its operands and binds its answer's variable.
                Op::Fn(op) => {
                    let of = |a: &V| if let V::Var(x) = *a { 1u64 << x } else { 0 };
                    n.needs = n.args[..op.operands()].iter().fold(0, |m, a| m | of(a));
                    n.binds = of(&n.args[op.operands()]) & !n.needs;
                }
            }
        }
        let fixed = t.steps.first().is_some_and(|s| s.side == Side::Delta);
        Some(ChainVars { nodes, fixed, out, level: t.level.clone() })
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The variables the atoms and operators of `placed` bind (an aggregate's answer, not its own variables).
    fn bound(&self, placed: u64) -> u64 {
        self.nodes.iter().enumerate().filter(|(j, _)| placed >> j & 1 == 1).fold(0, |m, (_, n)| match n.op {
            Op::Atom | Op::Fn(_) | Op::AggKeys => m | n.binds,
            Op::Agg(..) => m | (n.binds & if let Some(V::Var(x)) = n.args.last() { 1u64 << x } else { 0 }),
            _ => m,
        })
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
    /// What its relation declares, or its rules give it (0361): its keys and its columns' fanout caps.
    decl: Facts,
}

impl StepPrice {
    /// The rows a probe with columns `mask` bound returns at most, by its facts (0361).
    fn cap(&self, mask: usize) -> f64 {
        self.decl.cap(mask as u64)
    }
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
            s.decl.words(w);
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
                let corr = c.fs(m);
                StepPrice { card, ndv, corr, decl: Facts::of_words(&mut || c.next()) }
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
        Op::Atom | Op::AggKeys => {
            // An exact pair: b's bound variables all from one step a before it.
            let need: u64 = n.args.iter().filter_map(|a| if let V::Var(x) = a { (bound >> x & 1 == 1).then_some(1u64 << x) } else { None }).fold(0, |m, b| m | b);
            if need != 0
                && let Some(&(a, _, rows)) = lp.pairs.iter().find(|&&(a, b, _)| b == j && placed >> a & 1 == 1 && need & !c.nodes[a].binds == 0)
            {
                return (rows / lp.steps[a].card.max(1.0) * corr).min(sp.cap(mask));
            }
            (sp.card * sel * corr).min(sp.cap(mask))
        }
        Op::Not => (1.0 - (sp.card * sel).min(1.0)).max(0.01) * corr,
        Op::Cmp(cmp) => match cmp {
            Cmp::Eq => 0.1,
            Cmp::Ne => 0.9,
            _ => 1.0 / 3.0,
        },
        // One answer a tuple, or a test of the answer where it is bound.
        Op::Fn(_) | Op::Agg(..) => {
            if n.binds == 0 {
                0.1
            } else {
                1.0
            }
        }
    }
}

/// The rows a chain's first step makes: an atom's rows under its constants; a filter's one tuple, kept or not.
fn start_rows(c: &ChainVars, lp: &LinePrice, j: usize) -> f64 {
    match c.nodes[j].op {
        Op::Atom | Op::AggKeys => fan(c, lp, 0, j),
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
    /// The best order ending in each other step, cheapest first; a wider search's, its other proposals.
    runners: Vec<(Vec<usize>, f64)>,
    /// A wider search's (0203 §5): the order and runners are the line's proposals.
    wide: bool,
}

/// The cheapest valid order of chain `c` after its `prefix` (its steps already run, `rows_in` tuples there; empty: a
/// round's start, its fixed first step kept first), and the runners-up.
fn search(c: &ChainVars, lp: &LinePrice, prefix: &[usize], rows_in: f64, greedy: usize) -> Option<Searched> {
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
        Some(Searched { order, price, est, runners, wide: false })
    };
    if free == 0 {
        return done(prefix.clone(), Vec::new());
    }
    if free > greedy {
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
    let runners = ends[1..].iter().take(if greedy > GREEDY { PROPOSALS } else { RUNNERS_UP }).map(|&(_, l)| {
        let o = order_of(l);
        let p = price_of(&o);
        (o, p)
    });
    let runners: Vec<(Vec<usize>, f64)> = runners.collect();
    done(order_of(best), runners)
}

/// Up to `n` distinct random connected orders of chain `c` (0203 §5, `joins::random_orders`), drawn from `key`: each
/// next atom shares a variable with those placed while one can, a filter is placed as soon as it is ready, and a
/// fixed first step stays first.
fn random_orders(c: &ChainVars, n: usize, key: i64) -> Vec<Vec<usize>> {
    let len = c.len();
    let mut rng = SEED ^ key as u64;
    let mut below = |k: usize| {
        rng = splitmix(rng);
        (rng % k as u64) as usize
    };
    let mut out: Vec<Vec<usize>> = Vec::new();
    for _ in 0..n * 4 {
        if out.len() >= n {
            break;
        }
        let (mut order, mut placed) = (Vec::new(), 0u64);
        if c.fixed {
            order.push(0);
            placed |= 1;
        }
        while order.len() < len {
            let ready: Vec<usize> = (0..len).filter(|&j| placed >> j & 1 == 0 && c.ready(placed, j)).collect();
            if ready.is_empty() {
                break;
            }
            let pick = match ready.iter().copied().find(|&j| c.nodes[j].op != Op::Atom) {
                Some(f) => f,
                None => {
                    let joined: Vec<usize> = ready.iter().copied().filter(|&j| c.nodes[j].binds & c.bound(placed) != 0).collect();
                    let from = if joined.is_empty() { &ready } else { &joined };
                    from[below(from.len())]
                }
            };
            order.push(pick);
            placed |= 1 << pick;
        }
        if c.valid(&order) && !out.contains(&order) {
            out.push(order);
        }
    }
    out
}

/// The wider search of chain `c` (0203 §5): the exact search up to `WIDE_GREEDY` steps with its runners-up, and the
/// random connected orders, each priced; the best `PROPOSALS`, distinct, best first.
fn wide(c: &ChainVars, lp: &LinePrice, key: i64) -> Vec<(Vec<usize>, f64)> {
    let mut out: Vec<(Vec<usize>, f64)> = Vec::new();
    if let Some(s) = search(c, lp, &[], 0.0, WIDE_GREEDY) {
        out.push((s.order, s.price));
        out.extend(s.runners);
    }
    for o in random_orders(c, RANDOM_ORDERS, key) {
        let price = rows_along(c, lp, &o, 0, 0.0).iter().sum();
        out.push((o, price));
    }
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut seen: Vec<Vec<usize>> = Vec::new();
    out.retain(|(o, _)| {
        let new = !seen.contains(o);
        if new {
            seen.push(o.clone());
        }
        new
    });
    out.truncate(PROPOSALS);
    out
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
    /// Its share of the critical path, a moving average over runs (0203 §4).
    pub critical: f64,
    /// Up to `PROPOSALS` orders a wider search proposed, with their prices then (0203 §5).
    pub proposals: Vec<(Vec<usize>, f64)>,
    /// A wider search is wanted, not made yet.
    pub wants_search: bool,
    /// It ran since the run's critical path last marked it, and its work since then, over its rounds.
    pub fresh: bool,
    pub ran: f64,
    /// Its orders were trialled (0207 §2, 0274): once, and once more after it is flagged slow.
    pub trialled: bool,
    /// Its rows broke what its relations declare (0361): it is planned from statistics.
    pub broken: bool,
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
        w.extend([fb(self.critical), i64::from(self.wants_search), i64::from(self.fresh), fb(self.ran), i64::from(self.trialled), i64::from(self.broken), self.proposals.len() as i64]);
        for (o, x) in &self.proposals {
            w.extend(o.iter().map(|&j| j as i64));
            w.push(fb(*x));
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
        let (critical, wants_search, fresh, ran, trialled) = (c.f(), c.next() != 0, c.next() != 0, c.f(), c.next() != 0);
        let broken = c.next() != 0;
        let proposals = (0..c.n())
            .map(|_| {
                let o = c.vec(n);
                (o, c.f())
            })
            .collect();
        Note { order, price, est, work, start, runs, slow, tried, runners, pairs, samples, critical, proposals, wants_search, fresh, ran, trialled, broken }
    }

    /// Ratio `ratio` measured for order `o`, merged into its tried ratio, a geometric mean at weight `weight`; at most
    /// `TRIED` orders, the oldest dropped.
    fn merge_tried(&mut self, o: &[usize], ratio: f64, weight: f64) {
        let ratio = ratio.clamp(1e-6, 1e9);
        match self.tried.iter_mut().find(|t| t.0 == o) {
            Some(t) => t.1 = t.1.powf(1.0 - weight) * ratio.powf(weight),
            None => {
                if self.tried.len() >= TRIED {
                    self.tried.remove(0);
                }
                self.tried.push((o.to_vec(), ratio));
            }
        }
    }

    /// Whether it matters (0203 §7): on the critical path with its share, or slow.
    fn matters(&self) -> bool {
        self.critical >= CRITICAL || self.slow > 0
    }

    /// Whether it is pending (0203 §6): a wider search wanted, or a proposal not tried yet; searched at its next plan,
    /// its kept order not taken.
    fn pending(&self) -> bool {
        self.wants_search || self.proposals.iter().any(|(o, _)| self.ratio(o).is_none())
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
const DECLARED: i64 = 4;

/// A declared plan (0361): its order, its rows after each step at most, their sum, and each step's cap.
struct DeclaredPlan {
    order: Vec<usize>,
    est: Vec<f64>,
    price: f64,
    caps: Vec<f64>,
}

/// Chain `c`'s default plan from its relations' declarations alone (0361: no statistics, no search), its steps'
/// keys and caps `decl` and first steps' rows `rows` (the counts the run has exactly): its first step (its change,
/// else the atom of fewest rows), then while steps remain, each filter as soon as it is ready, else the atom whose
/// probe the declarations bound least (ties in rule order). `None` where an atom is left that no declaration bounds,
/// or none is placed by one (a line its facts do not plan).
fn declared_order(c: &ChainVars, decl: &[StepPrice], rows: impl Fn(usize) -> f64) -> Option<DeclaredPlan> {
    let n = c.len();
    let atoms: Vec<usize> = (0..n).filter(|&j| c.nodes[j].op == Op::Atom).collect();
    let first = if c.fixed { 0 } else { *atoms.iter().min_by(|&&a, &&b| rows(a).total_cmp(&rows(b)))? };
    let (mut order, mut placed) = (vec![first], 1u64 << first);
    let (mut est, mut caps) = (vec![rows(first)], vec![f64::INFINITY]);
    let mut by_facts = false;
    while order.len() < n {
        let ready: Vec<usize> = (0..n).filter(|&j| placed >> j & 1 == 0 && c.ready(placed, j)).collect();
        let (j, cap) = match ready.iter().copied().find(|&j| c.nodes[j].op != Op::Atom) {
            Some(f) => (f, if c.nodes[f].op == Op::Not { 1.0 } else { f64::INFINITY }),
            None => {
                let bound = c.bound(placed);
                let capped = ready.iter().map(|&j| (j, decl[j].cap(mask_of(&c.nodes[j], bound)))).filter(|x| x.1.is_finite());
                by_facts = true;
                capped.min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))?
            }
        };
        let last = *est.last().expect("a first step");
        est.push(if cap.is_finite() { last * cap } else { last });
        caps.push(cap);
        order.push(j);
        placed |= 1 << j;
    }
    let price = est.iter().sum();
    (by_facts && c.valid(&order)).then_some(DeclaredPlan { order, est, price, caps })
}

fn seed_draw(key: i64, runs: u64) -> f64 {
    let mut h = SEED ^ (key as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ runs.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Whether line `key`'s next plan may explore (0203 §7): it matters, and its seeded draw at its runs falls under
/// `EXPLORE_RATE`. Such a plan is searched, not kept, so the draw has candidates to take.
fn explores(n: &Note, key: i64) -> bool {
    n.matters() && seed_draw(key, n.runs) < EXPLORE_RATE
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
        // Its declarations (0361), unless its rows broke them.
        let decl: Vec<StepPrice> = t.steps.iter().map(|s| if matches!(s.op, Op::Cmp(_) | Op::Fn(_)) || note.as_ref().is_some_and(|n| n.broken) { StepPrice::default() } else { StepPrice { decl: env.declared(s.src), ..StepPrice::default() } }).collect();
        let mut default = None;
        let (status, kept) = match &chain {
            None => (ONE, Vec::new()),
            Some(ch) if ch.fixed && start(0) == 0.0 => (NONE, Vec::new()),
            Some(ch) if ch.len() - usize::from(ch.fixed) <= 1 => (ONE, Vec::new()),
            Some(ch) if let Some(d) = declared_order(ch, &decl, start) => {
                let o = d.order.clone();
                default = Some(d);
                (DECLARED, o)
            }
            Some(ch) => {
                let first = note.as_ref().map_or(0, |n| n.order[0]);
                let rows = start(if ch.fixed { 0 } else { first });
                match &note {
                    Some(n) if ch.valid(&n.order) && !n.pending() && !explores(n, keys[ti]) && n.work * (rows.max(1.0) / n.start.max(1.0)) < SEARCH_WORK => (KEPT, n.order.clone()),
                    _ => (PRICE, Vec::new()),
                }
            }
        };
        w.extend([status, kept.len() as i64]);
        w.extend(kept.iter().map(|&j| j as i64));
        if let Some(d) = default {
            w.push(fb(d.price));
            w.extend(d.est.iter().chain(&d.caps).map(|&x| fb(x)));
        }
    }
    put_words(out, &w);
}

struct LineInfo {
    status: i64,
    kept: Vec<usize>,
    /// A declared line's price, rows after each step and caps (0361).
    declared: Option<(f64, Vec<f64>, Vec<f64>)>,
}

fn lines_of(b: &[u8]) -> Vec<LineInfo> {
    let w = words_of(b);
    let mut c = Cur::new(&w);
    (0..c.n())
        .map(|_| {
            let status = c.next();
            let k = c.n();
            let kept = c.vec(k);
            let declared = (status == DECLARED).then(|| (c.f(), c.fs(k), c.fs(k)));
            LineInfo { status, kept, declared }
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
    /// A due point passed with its orders untrialled on the critical path (0207 §2, 0274): trialled at this depth;
    /// and whether this round trialled them.
    trial: bool,
    trialled: bool,
    /// Its rows broke a declaration (0361).
    broken: bool,
    counts: Vec<f64>,
}

fn running_of(b: &[u8]) -> Vec<RunningLine> {
    running_of_words(&words_of(b))
}

fn running_words(lines: &[RunningLine], width: usize) -> Vec<i64> {
    let mut w = vec![lines.len() as i64, width as i64];
    for l in lines {
        w.extend([fb(l.work), fb(l.due), fb(l.limit), i64::from(l.slow), l.cut as i64, l.times as i64, i64::from(l.trial), i64::from(l.trialled), i64::from(l.broken)]);
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
    /// A declared plan's (0361): the rows each step's probe returns at most per tuple in, by position (infinite:
    /// none declared), which `check` holds it to. Empty for any other.
    caps: Vec<f64>,
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
            let explored = c.next() != 0;
            let k = c.n();
            Choice { order, price, est, explored, caps: c.fs(k) }
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
        w.push(ch.caps.len() as i64);
        w.extend(ch.caps.iter().map(|&x| fb(x)));
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
        let note = Note::of(memo, keys[ti], t.steps.len());
        // A line whose declarations broke is priced by its statistics alone (0361: "or one whose rows broke what it
        // declared").
        let broken = note.as_ref().is_some_and(|n| n.broken) || rs.get(ti).is_some_and(|r| r.broken);
        let steps = t
            .steps
            .iter()
            .map(|s| {
                let (rows, ndv) = env.side_stats(s.src, s.side, s.args.len());
                let decl = if broken || matches!(s.op, Op::Cmp(_) | Op::Fn(_)) { Facts::default() } else { env.declared(s.src) };
                StepPrice { card: rows as f64, ndv: ndv.into_iter().map(|x| x as f64).collect(), corr: Vec::new(), decl }
            })
            .collect();
        let pairs = note.map(|n| n.pairs).unwrap_or_default();
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

/// `search`: read the plan, lines, prices, corrections, running, the running choice, memo · write the searched
/// orders. A round's line that wants a wider search, where its measured work is `WIDE_PAYS` times the search's cost,
/// has it inline (0203 §2, §5); any other's proposals are priced again beside its runners-up.
fn search_op(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, lines, prices, corr, running, now, memo], [out]) = c.views([0, 1, 2, 3, 4, 5, 6], [7]);
    let (p, keys) = plan_and_keys(plan);
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
                    search(&ch, lp, prefix, r.counts[r.cut], GREEDY)
                }
                None => {
                    let note = Note::of(memo, keys[ti], ch.len());
                    let free = ch.len() - usize::from(ch.fixed);
                    match note {
                        Some(n) if n.wants_search && n.work.max(n.ran) >= WIDE_PAYS * wide_cost(free) => {
                            let props = wide(&ch, lp, keys[ti]);
                            props.first().map(|(o, _)| {
                                let est = rows_along(&ch, lp, o, 0, 0.0);
                                Searched { order: o.clone(), price: est.iter().sum(), est, runners: props[1..].to_vec(), wide: true }
                            })
                        }
                        n => search(&ch, lp, &[], 0.0, GREEDY).map(|mut s| {
                            for (o, _) in n.map(|n| n.proposals).unwrap_or_default() {
                                if ch.valid(&o) && o != s.order && !s.runners.iter().any(|r| r.0 == o) {
                                    let price = rows_along(&ch, lp, &o, 0, 0.0).iter().sum();
                                    s.runners.push((o, price));
                                }
                            }
                            s
                        }),
                    }
                }
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
                w.push(i64::from(s.wide));
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
                let wide = c.next() != 0;
                Searched { order, price, est, runners, wide }
            })
        })
        .collect()
}

/// `choose`: read the plan, lines, search, memo, trials, the running choice · write the choice.
fn choose(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, lines, search, memo, trials, now], [out]) = c.views([0, 1, 2, 3, 4, 5], [6]);
    let (p, keys) = plan_and_keys(plan);
    let (ls, ss, now) = (lines_of(lines), searched_of(search), choices_of(now));
    // A handoff's trials: per line, each candidate's measured work per sample row (candidate 0: the running order);
    // `None` for a slow line not trialled, planned again from its cut (0274).
    let tw = words_of(trials);
    let mut measured: Vec<Option<Vec<f64>>> = Vec::new();
    if !tw.is_empty() {
        let mut tc = Cur::new(&tw);
        for _ in 0..tc.n() {
            let k = tc.next();
            measured.push((k >= 0).then(|| tc.fs(k as usize)));
        }
    }
    let mut cs = Vec::new();
    for (ti, t) in p.terms.iter().enumerate() {
        let rule: Vec<usize> = (0..t.steps.len()).collect();
        let note = Note::of(memo, keys[ti], t.steps.len());
        let ch = match (ss.get(ti).cloned().flatten(), now.get(ti)) {
            // A handoff: the order a trial measured least, where it beats the running one by the margin.
            (Some(s), Some(run)) if !now.is_empty() && matches!(measured.get(ti), Some(None)) => {
                if s.order == run.order { run.clone() } else { Choice { order: s.order.clone(), price: s.price, est: s.est.clone(), explored: false, caps: Vec::new() } }
            }
            (Some(s), Some(run)) if !now.is_empty() => {
                let m = measured.get(ti).cloned().flatten().unwrap_or_default();
                let cands: Vec<(Vec<usize>, f64)> = std::iter::once((run.order.clone(), run.price)).chain(std::iter::once((s.order.clone(), s.price))).chain(s.runners.iter().cloned()).collect();
                let best = (0..cands.len().min(m.len())).min_by(|&a, &b| m[a].total_cmp(&m[b])).unwrap_or(0);
                if best != 0 && m[best] * TRIAL_MARGIN < m[0] {
                    Choice { order: cands[best].0.clone(), price: cands[best].1, est: s.est.clone(), explored: false, caps: Vec::new() }
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
                    Some(u) if best.1 < EXPLORE_ROWS && note.as_ref().is_some_and(|n| n.matters()) && seed_draw(keys[ti], runs) < EXPLORE_RATE => Choice { order: u.0.clone(), price: u.1, est: s.est.clone(), explored: true, caps: Vec::new() },
                    _ => {
                        let est = if best.0 == s.order { s.est.clone() } else { vec![0.0; best.0.len()] };
                        Choice { order: best.0, price: best.1, est, explored: false, caps: Vec::new() }
                    }
                }
            }
            _ => match (ls[ti].status, &note) {
                (KEPT, Some(n)) => Choice { order: ls[ti].kept.clone(), price: n.price, est: n.est.clone(), explored: false, caps: Vec::new() },
                (DECLARED, _) => {
                    let (price, est, caps) = ls[ti].declared.clone().expect("a declared line's plan");
                    Choice { order: ls[ti].kept.clone(), price, est, explored: false, caps }
                }
                _ => Choice { order: rule.clone(), price: 0.0, est: vec![0.0; rule.len()], explored: false, caps: Vec::new() },
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
    let mut planned = Plan { srcs: p.srcs.clone(), docs: p.docs.clone(), decls: p.decls.clone(), terms: Vec::new() };
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
        note.runners = s.runners.iter().take(RUNNERS_UP).cloned().collect();
        if s.wide {
            note.proposals = std::iter::once((s.order.clone(), s.price)).chain(s.runners.iter().cloned()).take(PROPOSALS).collect();
            note.wants_search = false;
        }
        if !memo_twin::keep_line(c, 4, keys[ti] as u64, &note.words()) {
            return;
        }
    }
    let w: Vec<i64> = planned.words().into_iter().chain(std::iter::once(keys.len() as i64)).chain(keys.iter().copied()).collect();
    let ([], [out]) = c.views([], [3]);
    put_words(out, &w);
}

/// A step miss at depth `d` of a line that ran `counts` against its choice's priced rows `est` (0203 §3a), judged once
/// it has done `SLOW_FACTOR` searches' work: its widest step blown up (over `SLOW_FACTOR` times the widest priced, and
/// over a search's work), or a step over-estimated (priced at `SLOW_FACTOR` searches' work or more, entered with
/// `MISS_BINDINGS` bindings or more, it made under `1 / SLOW_FACTOR` of its priced rows per binding).
fn step_miss(work: f64, counts: &[f64], est: &[f64], d: usize) -> bool {
    if work < SLOW_FACTOR * SEARCH_WORK || est.len() != counts.len() || est.iter().all(|&x| x <= 0.0) {
        return false;
    }
    let widest = est.iter().copied().fold(0.0, f64::max);
    let blown = counts[d] > SLOW_FACTOR * widest && counts[d] > SEARCH_WORK;
    let over = d > 0 && est[d] >= SLOW_FACTOR * SEARCH_WORK && counts[d - 1] >= MISS_BINDINGS && est[d - 1] > 0.0 && counts[d] / counts[d - 1] < est[d] / est[d - 1] / SLOW_FACTOR;
    blown || over
}

/// `check`: read the plan, choice, frames, memo · rw running (its first word the depth just run): each line's tuples
/// counted at that depth; past its due point, slow when its work passed its limit or a step missed (0203 §3a), else
/// to be trialled where it is on the critical path with its orders untrialled (0207 §2, 0274).
fn check(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, choice, frames, memo], [running]) = c.views([0, 1, 2, 4], [3]);
    let (p, keys) = plan_and_keys(plan);
    let cs = choices_of(choice);
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
        // A declared plan's step past what its relation declares, at every depth (0361: "run time only checks that
        // the facts still hold"): the declaration broke, the line goes slow and is planned from statistics.
        let broke = d > 0 && !r.broken && cs.get(ti).and_then(|ch| ch.caps.get(d)).is_some_and(|&cap| cap.is_finite() && r.counts[d] > cap * r.counts[d - 1]);
        if broke {
            r.broken = true;
            if !r.slow {
                r.slow = true;
                r.times += 1;
                r.cut = d;
            }
        }
        if r.work >= r.due {
            while r.due <= r.work {
                r.due *= 2.0;
            }
            let missed = cs.get(ti).is_some_and(|ch| step_miss(r.work, &r.counts, &ch.est, d));
            if (r.work > r.limit || missed) && !r.slow {
                r.slow = true;
                r.times += 1;
                r.cut = d;
            } else if !r.slow && !r.trial && !r.trialled && Note::of(memo, keys[ti], p.terms[ti].steps.len()).is_some_and(|n| n.critical >= CRITICAL && !n.trialled) {
                r.trial = true;
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
    (0..n).map(|_| RunningLine { work: c.f(), due: c.f(), limit: c.f(), slow: c.next() != 0, cut: c.n(), times: c.next() as u64, trial: c.next() != 0, trialled: c.next() != 0, broken: c.next() != 0, counts: c.fs(width) }).collect()
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
            note.ran = if note.fresh { note.ran + r.work } else { r.work };
            note.fresh = true;
            // Trialled once, and once more after it is flagged slow (0207 §2).
            note.trialled = (note.trialled && r.times == 0) || r.trialled;
            note.broken |= r.broken;
            // A slow line asks for a wider search (0203 §3).
            if r.times > 0 {
                note.wants_search = true;
            }
            if ch.price > 0.0 {
                note.merge_tried(&ch.order, r.work.max(1.0) / ch.price.max(1.0), RUN_WEIGHT);
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

// ---- The run's critical path (0203 §4): over a level's strata, at the run's end.

rerail_delta_twins::params! {
    /// `weigh`'s and `mark`'s stratum: its place in the run's order.
    pub struct StratumP {
        pub at: u64,
    }
}

/// A path block's words: its strata's count, the path's work, the lines marked critical, then each stratum's weight,
/// then whether each is on the path, then each one's producers (a count, then their places, each before it).
const PATH_TOTAL: usize = 1;
const PATH_MARKED: usize = 2;
const PATH_AT: usize = 3;

/// A path block's words before `weigh`: strata `producers` (each stratum's, by place, in the run's order).
fn path_words(producers: &[Vec<usize>]) -> Vec<i64> {
    let n = producers.len();
    let mut w = vec![n as i64, fb(0.0), 0];
    w.extend(std::iter::repeat_n(fb(0.0), n));
    w.extend(std::iter::repeat_n(0, n));
    for ps in producers {
        w.push(ps.len() as i64);
        w.extend(ps.iter().map(|&p| p as i64));
    }
    w
}

/// `weigh`: read a stratum's memo · rw the path: the stratum's weight, the work its lines did this run.
fn weigh(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let at = <StratumP as rerail_delta_twins::Params>::read(c.params).at as usize;
    let ([memo], [path]) = c.views([0], [1]);
    let weight: f64 = notes(memo).iter().filter(|(_, n)| n.fresh).map(|(_, n)| n.ran).sum();
    let mut w = words_of(path);
    w[PATH_AT + at] = fb(weight);
    put_words(path, &w);
}

/// `critical`: rw the path: the longest chain of producers, by the strata's weights, marked on it; its work.
fn critical(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([], [path]) = c.views([], [0]);
    let mut w = words_of(path);
    let n = w[0] as usize;
    let weight: Vec<f64> = (0..n).map(|j| bf(w[PATH_AT + j])).collect();
    let mut at = PATH_AT + 2 * n;
    let (mut dist, mut prev) = (vec![0.0f64; n], vec![None::<usize>; n]);
    for j in 0..n {
        let np = w[at] as usize;
        let from = w[at + 1..at + 1 + np].iter().map(|&p| p as usize).filter(|&p| p < j).max_by(|&a, &b| dist[a].total_cmp(&dist[b]));
        at += 1 + np;
        dist[j] = weight[j] + from.map_or(0.0, |p| dist[p]);
        prev[j] = from;
    }
    let end = (0..n).max_by(|&a, &b| dist[a].total_cmp(&dist[b]));
    w[PATH_TOTAL] = fb(end.map_or(0.0, |e| dist[e]));
    let mut on = end;
    while let Some(j) = on {
        w[PATH_AT + n + j] = 1;
        on = prev[j];
    }
    put_words(path, &w);
}

/// `mark`: rw the path, a stratum's memo: each line that ran this run, its share of the path (its work over the
/// path's, where its stratum is on it; else 0) averaged into its note's; a wider search asked for where it is critical
/// (`CRITICAL` of the path, `CRITICAL_ROWS` of work) and its order measured over `CRITICAL_RATIO` times its price or it
/// has no proposals. Its run's work is let go.
fn mark(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let at = <StratumP as rerail_delta_twins::Params>::read(c.params).at as usize;
    let notes = {
        let ([], [path, memo]) = c.views([], [0, 1]);
        let w = words_of(path);
        let n = w[0] as usize;
        let (total, on) = (bf(w[PATH_TOTAL]), w[PATH_AT + n + at] == 1);
        let mut marked = 0i64;
        let mut out = Vec::new();
        for (key, mut note) in notes(memo).into_iter().filter(|(_, n)| n.fresh) {
            let share = if on && total > 0.0 { note.ran / total } else { 0.0 };
            note.critical = if note.runs <= 1 { share } else { 0.5 * note.critical + 0.5 * share };
            if share >= CRITICAL && note.ran >= CRITICAL_ROWS {
                marked += 1;
                if note.ratio(&note.order).is_some_and(|r| r > CRITICAL_RATIO) || note.proposals.is_empty() {
                    note.wants_search = true;
                }
            }
            note.fresh = false;
            note.ran = 0.0;
            out.push((key, note));
        }
        let mut w = w;
        w[PATH_MARKED] += marked;
        put_words(path, &w);
        out
    };
    for (key, note) in notes {
        if !memo_twin::keep_line(c, 1, key, &note.words()) {
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
static SEARCH_SLOTS: [SlotSpec; 8] = [s("plan", R, CTL), s("lines", R, CTL), s("prices", R, CTL), s("corrections", R, CTL), s("running", R, CTL), s("running choice", R, CTL), s("memo", R, kind::CACHE), s("searched", W, CTL)];
static CHOOSE_SLOTS: [SlotSpec; 7] = [s("plan", R, CTL), s("lines", R, CTL), s("searched", R, CTL), s("memo", R, kind::CACHE), s("trials", R, CTL), s("running choice", R, CTL), s("choice", W, CTL)];
static EMIT_SLOTS: [SlotSpec; 5] = [s("plan", R, CTL), s("choice", R, CTL), s("searched", R, CTL), s("planned", W, CTL), s("memo", RW, kind::CACHE)];
static CHECK_SLOTS: [SlotSpec; 5] = [s("plan", R, CTL), s("choice", R, CTL), s("frames", R, kind::SCRATCH), s("running", RW, CTL), s("memo", R, kind::CACHE)];
static STATS_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("atoms", R, kind::ATOMS), s("running", R, CTL), s("memo", RW, kind::CACHE)];
static NOTE_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("choice", R, CTL), s("running", R, CTL), s("memo", RW, kind::CACHE)];
static WEIGH_SLOTS: [SlotSpec; 2] = [s("memo", R, kind::CACHE), s("path", RW, CTL)];
static CRITICAL_SLOTS: [SlotSpec; 1] = [s("path", RW, CTL)];
static MARK_SLOTS: [SlotSpec; 2] = [s("path", RW, CTL), s("memo", RW, kind::CACHE)];

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

/// `weigh`'s slots' facts: its memo's (slot 0) by note, the path's by words.
fn memo0_facts(t: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 0 { memo_facts(bytes) } else { words_facts(t, slot, bytes) }
}

/// `mark`'s slots' facts: its memo's (slot 1) by note, the path's by words.
fn memo1_facts(t: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 1 { memo_facts(bytes) } else { words_facts(t, slot, bytes) }
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
const STRATA: Independence = Independence::Along("strata: a stratum's lines are its own");
pub static WEIGH: Signature = Signature::new("weigh", &WEIGH_SLOTS, ParamsLayout::of::<StratumP>(), &MAP, STRATA, memo0_facts);
pub static CRITICAL_OP: Signature = Signature::new("critical", &CRITICAL_SLOTS, ParamsLayout::of::<NoParams>(), &MAP, STRATA, words_facts);
pub static MARK: Signature = Signature::new("mark", &MARK_SLOTS, ParamsLayout::of::<StratumP>(), &MAP_MEMO, STRATA, memo1_facts);

static LINES_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: lines, again: None }];
static PRICE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: price, again: None }];
static SEARCH_HOST: [TwinPhase; 1] = [TwinPhase { name: "search", items: one, run: search_op, again: None }];
static CHOOSE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: choose, again: None }];
static EMIT_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: emit_plan, again: None }];
static CHECK_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: check, again: None }];
static STATS_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: stats, again: None }];
static NOTE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: note, again: None }];
static WEIGH_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: weigh, again: None }];
static CRITICAL_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: critical, again: None }];
static MARK_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: mark, again: None }];

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
    pub weigh: OpId,
    pub critical: OpId,
    pub mark: OpId,
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
            weigh: d(&WEIGH, "weigh on the host", &WEIGH_HOST, &MAP)?,
            critical: d(&CRITICAL_OP, "critical on the host", &CRITICAL_HOST, &MAP)?,
            mark: d(&MARK, "mark on the host", &MARK_HOST, &MAP_MEMO)?,
        })
    }

    /// The run's critical path (0203 §4) over strata `memos` (each stratum's memo, in the run's order) with
    /// `producers` (each one's, by place, each before it): `weigh` each, `critical`, then `mark` each. The lines it
    /// marked critical.
    pub fn critical_path(&self, reg: &Registry, dir: &mut Directory, memos: &[BlockId], producers: &[Vec<usize>]) -> Result<u64, String> {
        if memos.is_empty() {
            return Ok(0);
        }
        let path = given(dir, &path_words(producers))?;
        let at = |j: usize| {
            let mut p = vec![0u8; <StratumP as rerail_delta_twins::Params>::SIZE];
            rerail_delta_twins::Params::write(&StratumP { at: j as u64 }, &mut p);
            p
        };
        let ran = (|| {
            for (j, &m) in memos.iter().enumerate() {
                run_cpu(reg, dir, self.weigh, &[m, path], &at(j), Tiling::WHOLE)?;
            }
            run_cpu(reg, dir, self.critical, &[path], &[], Tiling::WHOLE)?;
            for (j, &m) in memos.iter().enumerate() {
                run_cpu(reg, dir, self.mark, &[path, m], &at(j), Tiling::WHOLE)?;
            }
            read_block(dir, path, |b| words_of(b)[PATH_MARKED] as u64)
        })();
        dir.release(path);
        ran
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

/// A round's run: the probes made, what the planning did, and the declared facts the rows broke (each relation and
/// the columns its broken probe bound, a mask).
pub type Ran = (u64, Planned, Vec<(i64, u64)>);

impl Round<'_> {
    /// Plans `plan`'s chains (their steps in their rules' order, each chain's line key in `keys`) and runs them into
    /// the heads' runs `runs`, checking each depth: the probes made, what the planning did, and the declared facts
    /// the rows broke (each relation, and the columns its broken probe bound).
    pub fn run(&self, dir: &mut Directory, plan: &Plan, keys: &[i64], runs: &[BlockId]) -> Result<Ran, String> {
        if plan.terms.is_empty() {
            return Ok((0, Planned::default(), Vec::new()));
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

    fn run_in(&self, dir: &mut Directory, plan: &Plan, keys: &[i64], runs: &[BlockId], made: &mut Vec<BlockId>) -> Result<Ran, String> {
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
        pd.declared += ls.iter().filter(|l| l.status == DECLARED).count() as u64;
        let any = ls.iter().any(|l| l.status == PRICE);
        let searched = if any {
            let prices = self.into(dir, made, self.ops.price, &[rule, atoms, memo, lines, empty], 5, n)?;
            pd.priced += read_block(dir, prices, |b| prices_of(b).iter().flatten().map(|lp| lp.steps.len() as u64).sum::<u64>())?;
            let corr = self.correct(dir, made, prices)?;
            let searched = self.into(dir, made, self.ops.search, &[rule, lines, prices, corr, empty, empty, memo], 7, n)?;
            let ss = read_block(dir, searched, searched_of)?;
            pd.searched += ss.iter().flatten().count() as u64;
            pd.wide += ss.iter().flatten().filter(|s| s.wide).count() as u64;
            searched
        } else {
            let none = given(dir, &[0])?;
            made.push(none);
            none
        };
        let choice = self.into(dir, made, self.ops.choose, &[rule, lines, searched, memo, empty, empty], 6, n)?;
        pd.explored += read_block(dir, choice, |b| choices_of(b).iter().filter(|c| c.explored).count() as u64)?;
        let planned_b = self.planned(dir, made, rule, choice, searched, n)?;
        let planned = read_block(dir, planned_b, |b| plan_and_keys(b).0)?;
        // The run, checked after each depth.
        let width = planned.width();
        let cs = read_block(dir, choice, choices_of)?;
        let start: Vec<RunningLine> = cs.iter().map(|c| RunningLine { work: 0.0, due: SLOW_ROWS, limit: (SLOW_FACTOR * c.price).max(SLOW_ROWS), slow: false, cut: 0, times: 0, trial: false, trialled: false, broken: false, counts: vec![0.0; width] }).collect();
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
                self.launch(dir, self.ops.check, &[pb, choice, frames, running, memo])?;
                let rs = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
                let here = |r: &RunningLine| r.cut == d && d + 1 < width;
                let slow: Vec<usize> = rs.iter().enumerate().filter(|(_, r)| r.slow && here(r)).map(|(ti, _)| ti).collect();
                let trial: Vec<usize> = rs.iter().enumerate().filter(|(_, r)| r.trial && !r.slow && here(r)).map(|(ti, _)| ti).collect();
                if slow.is_empty() && trial.is_empty() {
                    return Ok(pb);
                }
                pd.slow += slow.len() as u64;
                Ok(match self.handoff(dir, made, rule, choice, running, frames, &slow, &trial, d, n, &mut pd)? {
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
        // The facts broken: a line's step at its cut, the prefix a handoff keeps, its relation and its bound columns.
        let rs = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
        let mut broken = Vec::new();
        for (ti, r) in rs.iter().enumerate().filter(|(_, r)| r.broken) {
            pd.broken += 1;
            let st = &planned.terms[ti].steps[r.cut];
            let rel = planned.decls.rels.get(st.src).copied().unwrap_or(0);
            let mask = st.args.iter().enumerate().filter(|(_, a)| matches!(a, Arg::Eq(..) | Arg::Const(_))).fold(0u64, |m, (i, _)| m | 1 << i.min(63));
            if !broken.contains(&(rel, mask)) {
                broken.push((rel, mask));
            }
        }
        self.launch(dir, self.ops.note, &[rule, choice, running, memo])?;
        Ok((probes, pd, broken))
    }

    /// `emit_plan` into a new plan block, which then refers to every block its chains read and is frozen.
    fn planned(&self, dir: &mut Directory, made: &mut Vec<BlockId>, rule: BlockId, choice: BlockId, searched: BlockId, n: usize) -> Result<BlockId, String> {
        let out = words_block(dir, n);
        made.push(out);
        self.launch(dir, self.ops.emit, &[rule, choice, searched, out, self.memo])?;
        let p = read_block(dir, out, |b| plan_and_keys(b).0)?;
        dir.refer_all(out, &p.reads())?;
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

    /// A handoff (0206) at depth `d` for the slow lines `slow` and the lines `trial` whose orders are due a trial
    /// (0207 §2): the slow lines' exact pairs; their prices and searches from the steps they ran; their candidates
    /// trialled over a sample of their tuples there where they are on the critical path (0274), a slow line off it
    /// planned again from its cut; the choice, and a new plan block when any line's order changed (`None`: none did).
    #[allow(clippy::too_many_arguments)]
    fn handoff(&self, dir: &mut Directory, made: &mut Vec<BlockId>, rule: BlockId, choice: BlockId, running: BlockId, frames: BlockId, slow: &[usize], trial: &[usize], d: usize, n: usize, pd: &mut Planned) -> Result<Option<(BlockId, BlockId)>, String> {
        let (atoms, memo) = (self.atoms, self.memo);
        let handed: Vec<usize> = slow.iter().chain(trial).copied().collect();
        // The running block as `price` and `search` read it: without its depth word.
        let rs = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
        let width = rs.first().map_or(0, |r| r.counts.len());
        let marked = |of: &[usize]| -> Vec<RunningLine> { rs.iter().enumerate().map(|(ti, r)| RunningLine { slow: of.contains(&ti), cut: d, counts: r.counts.clone(), ..*r }).collect() };
        let rblock = given(dir, &running_words(&marked(&handed), width))?;
        made.push(rblock);
        let mut sw = vec![0i64];
        sw.extend(running_words(&marked(slow), width));
        let sblock = given(dir, &sw)?;
        made.push(sblock);
        self.launch(dir, self.ops.stats, &[rule, atoms, sblock, memo])?;
        // The pairs measured for the slow lines.
        let (p, keys) = read_block(dir, rule, plan_and_keys)?;
        let slow_keys: Vec<u64> = slow.iter().map(|&ti| keys[ti] as u64).collect();
        pd.pairs += read_block(dir, memo, |b| notes(b).iter().filter(|(k, _)| slow_keys.contains(k)).map(|(_, n)| n.pairs.len() as u64).sum::<u64>())?;
        let mut lw = vec![p.terms.len() as i64];
        for _ in &p.terms {
            lw.extend([PRICE, 0]);
        }
        let lines = {
            let b = given(dir, &lw)?;
            made.push(b);
            b
        };
        let prices = self.into(dir, made, self.ops.price, &[rule, atoms, memo, lines, rblock], 5, n)?;
        let corr = self.correct(dir, made, prices)?;
        let searched = self.into(dir, made, self.ops.search, &[rule, lines, prices, corr, rblock, choice, self.memo], 7, n)?;
        // The trials: each line's running order and its candidates over a sample of its tuples at depth `d`, where it
        // is on the critical path (0274). A slow line not trialled is planned again (-1: the search's best from its cut).
        let ss = read_block(dir, searched, searched_of)?;
        let cs = read_block(dir, choice, choices_of)?;
        let on_path: Vec<bool> = read_block(dir, memo, |b| p.terms.iter().enumerate().map(|(ti, t)| Note::of(b, keys[ti], t.steps.len()).is_some_and(|n| n.critical >= CRITICAL)).collect())?;
        let mut tried_lines: Vec<usize> = Vec::new();
        let mut tw = vec![p.terms.len() as i64];
        for (ti, t) in p.terms.iter().enumerate() {
            let untried = if slow.contains(&ti) { -1 } else { 0 };
            let (Some(Some(s)), true) = (ss.get(ti), handed.contains(&ti)) else {
                tw.push(0);
                continue;
            };
            let here = rs[ti].counts[d] as usize;
            let share = (TRIAL_SHARE * rs[ti].work / (rs[ti].work / here.max(1) as f64).max(1.0)) as usize;
            let sample = here.min(TRIAL_ROWS).min(share.max(TRIAL_LEAST));
            if !on_path[ti] || sample < TRIAL_LEAST || (s.order == cs[ti].order && s.runners.is_empty()) {
                tw.push(untried);
                continue;
            }
            tried_lines.push(ti);
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
                let w = self.trial(dir, &p, ti, &ch, o, frames, d, sample, TRIAL_CUTOFF * least * sample as f64)?;
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
        // A line due a trial was handed off once at this depth; a line trialled keeps it for its note.
        {
            let mut rs_now = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
            for (ti, r) in rs_now.iter_mut().enumerate() {
                r.trial = r.trial && !trial.contains(&ti);
                r.trialled |= tried_lines.contains(&ti);
            }
            let mut out = vec![d as i64];
            out.extend(running_words(&rs_now, width));
            let b = dir.block(running).expect("made");
            let mut pin = pin_block(&b, "the running lines", true)?;
            put_words(pin.bytes_mut().expect("a write pin"), &out);
        }
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
