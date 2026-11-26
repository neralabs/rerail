//! A rule body as operations (decision 0686, O1 as settled; *0686: Operations* §10, "A rule body"; *Steps, Strata,
//! Roll-up* §1): `seed`, `step` and `head` over frames, each an operation over blocks with its host version
//! registered as its reference.
//!
//! A **plan** is a stratum's chains as data (`Plan`), written into a plan block that refers to every block a chain
//! reads: each relation's two states (a trace's heads, its layers below reached by reference), and each document's
//! source, tape and symbols where they lie. A chain (`TermPlan`) is a list of steps, so that a fused form can later
//! be a rewrite of it, checked against it, not a second definition. A **frame** is the tuples of row ids a chain has
//! matched so far, one column a step, with a sign: a variable's value is never copied into it, it is read through the
//! row id of the step that bound it (`Arg::Eq`). Only the head writes.
//!
//! - **seed**: every chain's start, its first step's rows: a side's live rows (`Side::After` or `Before`, a cold
//!   chain), or its change (`Side::Delta`: gone −1, added +1; for a `not`, each key whose absence changed, its sign
//!   "none after" − "none before"), restricted to the chain's partitions in a single-file wave;
//! - **step** (a depth a launch): each tuple through its chain's step at that depth: an atom joins (probing the side
//!   it reads on the bound columns, B§3 sentence 2), a `not` keeps the tuples with no match, a `cmp` filters; a tuple
//!   whose chain is shorter passes on;
//! - **head** (a head relation a launch): each finished tuple's head row, read through its row ids, pushed with its
//!   sign into the head's δ runs (`tally` counts them, `tally`).
//!
//! Each launch counts what it emits and then places it (an emit, Operations §4, shape 11): past its out block's room
//! it writes nothing and names the room it needs in control, and the host runs it again at that room. A row id in a
//! frame is a row of its step's source on one side: bit 63 set for the before side; for a view, the document's place
//! in the plan in bits 32 to 62 and its node below.

use crate::dir::{BlockId, Directory};
use crate::files::Placed;
use crate::op::{Across, Bound, Found, Independence, Inside, Mode, OpId, Output, Over, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use crate::program::Cmp;
use crate::shapes::Facts;
use crate::sym::Names;
use crate::tape::{Reads, TapeView, View};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, set_word, word};
use rerail_blocks::layer::{Runs, runs_claim_in, runs_set_in};
use rerail_blocks::region::pin_block;
use rerail_blocks::table::Table as _;
use rerail_delta_twins::{Ctx, NoParams, Tile, TwinPhase};
use std::collections::BTreeSet;

/// Which state of its source a step reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Before = 0,
    After = 1,
    /// The change between the two (a chain's first step only).
    Delta = 2,
}

/// A document's text, where it lies: its source, its tape and its symbols block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    pub src: Placed,
    pub tape: Placed,
    pub syms: BlockId,
}

/// A document of a plan: its key, and its text on each side (`None`: not in that state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocAt {
    pub d: i64,
    pub before: Option<Text>,
    pub after: Option<Text>,
}

/// Where a step's rows come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Src {
    /// A trace between two heads (`None`: ∅), of `arity` columns.
    Trace { arity: usize, before: Option<BlockId>, after: Option<BlockId> },
    /// A view of the plan's documents at these places.
    View { view: View, docs: Vec<usize> },
}

/// A step's argument as the chain binds it: a variable bound here (`Bind`), `_`, a constant, a variable bound by an
/// earlier step of the chain (`Eq(step, column)`, read through that step's row id), or one bound by this same row
/// (`Same(column)`). A `cmp` step's two arguments and a head's are `Const` or `Eq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    Bind,
    Wild,
    Const(i64),
    Eq(usize, usize),
    Same(usize),
}

/// What a step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Atom,
    Not,
    Cmp(Cmp),
}

/// A step of a chain: what it does, its source and side, its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepPlan {
    pub op: Op,
    pub src: usize,
    pub side: Side,
    pub args: Vec<Arg>,
}

/// A chain: the head it writes (its place in the plan's heads), the sign of its rows (−1: a removed rule's rows), the
/// partitions it starts in (none: all), its steps in the order they run, and the head's arguments. A recursive
/// stratum's chain names `level`, the (step, column) of each level its derivation reads (0364 §2.4): the head then
/// writes one more column, one more than the highest of them (0 with none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermPlan {
    pub head: usize,
    pub sign: i64,
    pub parts: Vec<i64>,
    pub steps: Vec<StepPlan>,
    pub out: Vec<Arg>,
    pub level: Option<Vec<(usize, usize)>>,
}

/// The shapes a program's relations declare (decision 0361), as the planner reads them: the sources of the
/// declarations' relations (`shapes::DECLS`: `key`, `fanout`, `tree`, `order`), read where they lie; each source's
/// relation (0: none, a view's own); the symbols of `one` and `few`; and the facts inferred for the derived relations
/// it reads (`shapes::infer`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decls {
    pub srcs: [Option<usize>; 4],
    pub rels: Vec<i64>,
    pub one: i64,
    pub few: i64,
    pub inferred: Vec<(i64, Facts)>,
}

/// At most this many rows a probe of a `few` fanout returns (0361).
pub const FEW: f64 = 16.0;

/// A stratum's chains as data, with the sources and documents they read, and the shapes its relations declare.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub srcs: Vec<Src>,
    pub docs: Vec<DocAt>,
    pub decls: Decls,
    pub terms: Vec<TermPlan>,
}

const BEFORE: u64 = 1 << 63;
/// A tuple's place for a step that binds no row (a fact's, a leading filter's): never read, as no argument names it.
const NO_ROW: u64 = u64::MAX;
const DOC_SHIFT: u32 = 32;

fn rref(before: bool, doc: usize, id: u32) -> u64 {
    (if before { BEFORE } else { 0 }) | ((doc as u64) << DOC_SHIFT) | u64::from(id)
}

fn unref(r: u64) -> (bool, usize, u32) {
    (r & BEFORE != 0, ((r & !BEFORE) >> DOC_SHIFT) as usize, r as u32)
}

// The plan's words.
const PLAN_MAGIC: i64 = i64::from_le_bytes(*b"PLAN0686");
const NO: i64 = -1;

fn put_block(w: &mut Vec<i64>, b: Option<BlockId>) {
    w.push(b.map_or(NO, |b| i64::from(b.0)));
}

fn put_placed(w: &mut Vec<i64>, p: &Placed) {
    w.extend([i64::from(p.block.0), p.at as i64, p.len as i64]);
}

fn put_arg(w: &mut Vec<i64>, a: &Arg) {
    let (t, x, y) = match *a {
        Arg::Bind => (0, 0, 0),
        Arg::Wild => (1, 0, 0),
        Arg::Const(v) => (2, v, 0),
        Arg::Eq(j, c) => (3, j as i64, c as i64),
        Arg::Same(c) => (4, c as i64, 0),
    };
    w.extend([t, x, y]);
}

impl Plan {
    /// Its words, as the plan block holds them.
    pub fn words(&self) -> Vec<i64> {
        let mut w = vec![PLAN_MAGIC, self.srcs.len() as i64];
        for s in &self.srcs {
            match s {
                Src::Trace { arity, before, after } => {
                    w.extend([0, *arity as i64]);
                    put_block(&mut w, *before);
                    put_block(&mut w, *after);
                }
                Src::View { view, docs } => {
                    w.extend([1, View::ALL.iter().position(|v| v.0 == *view).expect("a view") as i64, docs.len() as i64]);
                    w.extend(docs.iter().map(|&d| d as i64));
                }
            }
        }
        w.push(self.docs.len() as i64);
        for d in &self.docs {
            w.push(d.d);
            for t in [d.before, d.after] {
                match t {
                    None => w.extend([NO; 7]),
                    Some(t) => {
                        put_placed(&mut w, &t.src);
                        put_placed(&mut w, &t.tape);
                        w.push(i64::from(t.syms.0));
                    }
                }
            }
        }
        w.push(self.terms.len() as i64);
        for t in &self.terms {
            w.extend([t.head as i64, t.sign, t.parts.len() as i64]);
            w.extend(&t.parts);
            w.push(t.steps.len() as i64);
            for s in &t.steps {
                let op = match s.op {
                    Op::Atom => 0,
                    Op::Not => 1,
                    Op::Cmp(c) => 2 + c.code(),
                };
                w.extend([op, s.src as i64, s.side as i64, s.args.len() as i64]);
                s.args.iter().for_each(|a| put_arg(&mut w, a));
            }
            w.push(t.out.len() as i64);
            t.out.iter().for_each(|a| put_arg(&mut w, a));
            match &t.level {
                None => w.push(-1),
                Some(l) => {
                    w.push(l.len() as i64);
                    w.extend(l.iter().flat_map(|&(j, c)| [j as i64, c as i64]));
                }
            }
        }
        let d = &self.decls;
        w.extend(d.srcs.map(|x| x.map_or(-1, |i| i as i64)));
        w.extend([d.one, d.few, d.rels.len() as i64]);
        w.extend(&d.rels);
        w.push(d.inferred.len() as i64);
        for (rel, f) in &d.inferred {
            w.push(*rel);
            f.words(&mut w);
        }
        w
    }

    /// The plan its words hold.
    pub fn of_words(w: &[i64]) -> Plan {
        let mut at = 0;
        let mut next = || {
            at += 1;
            w[at - 1]
        };
        assert_eq!(next(), PLAN_MAGIC, "a plan block");
        let block = |x: i64| (x >= 0).then_some(BlockId(x as u32));
        let mut p = Plan::default();
        for _ in 0..next() {
            p.srcs.push(match next() {
                0 => {
                    let arity = next() as usize;
                    let before = block(next());
                    Src::Trace { arity, before, after: block(next()) }
                }
                _ => {
                    let view = View::ALL[next() as usize].0;
                    let n = next();
                    Src::View { view, docs: (0..n).map(|_| next() as usize).collect() }
                }
            });
        }
        for _ in 0..next() {
            let d = next();
            let mut text = || {
                let s: [i64; 7] = std::array::from_fn(|_| next());
                (s[0] >= 0).then(|| Text {
                    src: Placed { block: BlockId(s[0] as u32), at: s[1] as usize, len: s[2] as usize },
                    tape: Placed { block: BlockId(s[3] as u32), at: s[4] as usize, len: s[5] as usize },
                    syms: BlockId(s[6] as u32),
                })
            };
            let before = text();
            p.docs.push(DocAt { d, before, after: text() });
        }
        let arg = |next: &mut dyn FnMut() -> i64| {
            let (t, x, y) = (next(), next(), next());
            match t {
                0 => Arg::Bind,
                1 => Arg::Wild,
                2 => Arg::Const(x),
                3 => Arg::Eq(x as usize, y as usize),
                _ => Arg::Same(x as usize),
            }
        };
        for _ in 0..next() {
            let (head, sign, np) = (next() as usize, next(), next());
            let parts = (0..np).map(|_| next()).collect();
            let mut steps = Vec::new();
            for _ in 0..next() {
                let op = match next() {
                    0 => Op::Atom,
                    1 => Op::Not,
                    c => Op::Cmp(Cmp::of_code(c - 2).expect("an operator")),
                };
                let (src, side) = (next() as usize, [Side::Before, Side::After, Side::Delta][next() as usize]);
                let n = next();
                let args = (0..n).map(|_| arg(&mut next)).collect();
                steps.push(StepPlan { op, src, side, args });
            }
            let n = next();
            let out = (0..n).map(|_| arg(&mut next)).collect();
            let nl = next();
            let level = (nl >= 0).then(|| (0..nl).map(|_| (next() as usize, next() as usize)).collect());
            p.terms.push(TermPlan { head, sign, parts, steps, out, level });
        }
        let srcs = [(); 4].map(|()| usize::try_from(next()).ok());
        let (one, few) = (next(), next());
        let rels = (0..next()).map(|_| next()).collect();
        let inferred = (0..next()).map(|_| (next(), Facts::of_words(&mut next))).collect();
        p.decls = Decls { srcs, rels, one, few, inferred };
        p
    }

    /// The blocks it reads, each once, with the offset a reference to it names.
    pub(crate) fn reads(&self) -> Vec<(BlockId, usize)> {
        let mut r: BTreeSet<(u32, usize)> = BTreeSet::new();
        for s in &self.srcs {
            if let Src::Trace { before, after, .. } = s {
                r.extend([before, after].into_iter().flatten().map(|b| (b.0, BODY_AT)));
            }
        }
        for t in self.docs.iter().flat_map(|d| [d.before, d.after]).flatten() {
            r.extend([(t.src.block.0, t.src.at), (t.tape.block.0, t.tape.at), (t.syms.0, BODY_AT)]);
        }
        r.into_iter().map(|(b, at)| (BlockId(b), at)).collect()
    }

    /// The longest chain's steps: the frames' width and the step launches' count.
    pub fn width(&self) -> usize {
        self.terms.iter().map(|t| t.steps.len()).max().unwrap_or(0).max(1)
    }
}

/// A source as a launch reads it: a trace's two states, or a view's documents' two sides.
pub(crate) enum Live<'t> {
    Trace { before: rerail_blocks::relation::View<'t>, after: rerail_blocks::relation::View<'t> },
    View { docs: Vec<usize> },
}

/// Everything a launch reads, where it lies: the plan, its sources, and its documents' views.
pub(crate) struct Env<'t> {
    pub(crate) plan: Plan,
    srcs: Vec<Live<'t>>,
    /// Each document's view of each kind on each side, by its place: `[before, after]`.
    views: Vec<[Vec<Option<TapeView<'t>>>; 2]>,
}

impl<'t> Env<'t> {
    pub(crate) fn new(table: &'t Table<'t>, plan_bytes: &[u8], names: &'t Names) -> Env<'t> {
        let n = word::<i64>(plan_bytes, BODY_AT) as usize;
        let words: Vec<i64> = (0..n).map(|i| word::<i64>(plan_bytes, BODY_AT + 8 + 8 * i)).collect(); // copy: derived (the plan's few words, decoded)
        let plan = Plan::of_words(&words);
        let srcs = plan
            .srcs
            .iter()
            .map(|s| match s {
                Src::Trace { arity, before, after } => {
                    let st = |h: &Option<BlockId>| rerail_blocks::relation::View::chain(table, h.map(|b| b.0), *arity).expect("a trace's state, reached by reference");
                    Live::Trace { before: st(before), after: st(after) }
                }
                Src::View { docs, .. } => Live::View { docs: docs.clone() },
            })
            .collect();
        let text = |t: &Option<Text>, d: i64| -> Vec<Option<TapeView<'t>>> {
            let Some(t) = t else { return View::ALL.iter().map(|_| None).collect() };
            let blk = |b: BlockId| table.block(b.0).expect("a document's block, reached by reference");
            let src = std::str::from_utf8(&blk(t.src.block)[t.src.at..t.src.at + t.src.len]).expect("a source the file layer checked is UTF-8");
            let reads = Reads::in_place(&blk(t.tape.block)[t.tape.at..t.tape.at + t.tape.len], src).expect("a tape the parser wrote");
            let syms = blk(t.syms);
            View::ALL.iter().map(|v| Some(TapeView::new(v.0, d, reads, syms, names))).collect()
        };
        let views = plan.docs.iter().map(|d| [text(&d.before, d.d), text(&d.after, d.d)]).collect();
        Env { plan, srcs, views }
    }

    fn view_of(&self, src: usize) -> View {
        match &self.plan.srcs[src] {
            Src::View { view, .. } => *view,
            Src::Trace { .. } => unreachable!("a trace"),
        }
    }

    fn tape(&self, src: usize, doc: usize, before: bool) -> Option<&TapeView<'t>> {
        let v = View::ALL.iter().position(|x| x.0 == self.view_of(src)).expect("a view");
        self.views[doc][usize::from(!before)][v].as_ref()
    }

    /// Column `c` of the row `r` refers to in source `src`.
    fn get(&self, src: usize, r: u64, c: usize) -> i64 {
        let (before, doc, id) = unref(r);
        let c = ColumnIndex::new(c as u16);
        match &self.srcs[src] {
            Live::Trace { before: b, after: a } => Read::get(if before { b } else { a }, RowId::new(id), c),
            Live::View { .. } => Read::get(self.tape(src, doc, before).expect("a document on that side"), RowId::new(id), c),
        }
    }

    /// Each live row of `src`'s side `before` whose columns `cols` hold `key`, until `f` says stop: whether it stopped.
    fn each(&self, src: usize, before: bool, cols: &[ColumnIndex], key: &[i64], f: &mut dyn FnMut(u64) -> bool) -> bool {
        match &self.srcs[src] {
            Live::Trace { before: b, after: a } => {
                let s = if before { b } else { a };
                for id in Read::matching(s, cols, key) {
                    if !f(rref(before, 0, id.get())) {
                        return true;
                    }
                }
                false
            }
            Live::View { docs } => {
                for &doc in docs {
                    let Some(t) = self.tape(src, doc, before) else { continue };
                    let go = t.probe(cols, key, &mut |id| f(rref(before, doc, id.get())));
                    if !go {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Source `src`'s rows on `side` (its change's, for `Delta`), and each of its first `ncols` columns' distinct values
    /// (*0686: Operations* §8's statistics, read where they lie: a trace's from its layers' sketches, a view's from its
    /// documents' rows).
    /// What source `src`'s relation declares (0361), read where the declarations lie, with what its rules give it
    /// (`shapes::infer`): its keys, each a mask of its columns, and its columns' fanout caps (the rows a probe binding
    /// the column returns at most).
    pub(crate) fn declared(&self, src: usize) -> Facts {
        let d = &self.plan.decls;
        let Some(&rel) = d.rels.get(src).filter(|&&r| r != 0) else { return Facts::default() };
        let rows = |at: Option<usize>| -> BTreeSet<Vec<i64>> {
            match at.map(|i| &self.srcs[i]) {
                Some(Live::Trace { after, .. }) => {
                    let c = |i: u16| ColumnIndex::new(i);
                    Read::matching(after, &[c(0)], &[rel]).map(|r| vec![Read::get(after, r, c(0)), Read::get(after, r, c(1)), Read::get(after, r, c(2))]).collect() // copy: derived (a relation's few declarations, read)
                }
                _ => BTreeSet::new(),
            }
        };
        let own = crate::shapes::declared(&d.srcs.map(rows), Some(d.one), Some(d.few)).remove(&rel).unwrap_or_default();
        d.inferred.iter().filter(|i| i.0 == rel).fold(own, |f, i| f.and(&i.1))
    }

    pub(crate) fn side_stats(&self, src: usize, side: Side, ncols: usize) -> (u64, Vec<u64>) {
        let of = |v: &dyn Fn(ColumnIndex) -> rerail_blocks::bistate::ColStats, rows: &dyn Fn() -> u64| -> (u64, Vec<u64>) {
            let n = if ncols == 0 { rows() } else { v(ColumnIndex::new(0)).rows };
            (n, (0..ncols).map(|c| v(ColumnIndex::new(c as u16)).ndv.max(1)).collect())
        };
        match &self.srcs[src] {
            Live::Trace { before, after } => {
                let v = if side == Side::Before { before } else { after };
                let (rows, ndv) = of(&|c| Read::stats(v, c), &|| Read::live_ids(v).count() as u64);
                if side == Side::Delta {
                    let (g, a) = Bistate::new(*before, *after).counts();
                    let n = (g + a) as u64;
                    return (n, ndv.into_iter().map(|d| d.min(n.max(1))).collect());
                }
                (rows, ndv)
            }
            Live::View { docs } => {
                let sides: &[bool] = if side == Side::Delta { &[true, false] } else { &[side == Side::Before] };
                let (mut rows, mut ndv) = (0u64, vec![0u64; ncols]);
                for &doc in docs {
                    for &before in sides {
                        let Some(t) = self.tape(src, doc, before) else { continue };
                        let (r, d) = of(&|c| Read::stats(t, c), &|| Read::live_ids(t).count() as u64);
                        rows += r;
                        ndv.iter_mut().zip(d).for_each(|(x, y)| *x += y);
                    }
                }
                (rows, ndv.into_iter().map(|d| d.clamp(1, rows.max(1))).collect())
            }
        }
    }

    /// The exact rows of step `a` joined with step `b` on the columns `on` (`(a's column, b's column)`), each on its
    /// side, `a`'s constants applied (0203 §3's exact pair statistics, for a slow line): `a`'s rows read, `b` probed.
    pub(crate) fn pair_rows(&self, a: &StepPlan, b: &StepPlan, on: &[(usize, usize)]) -> u64 {
        // Each step's own constants (a column bound by another step of the chain is free here: the pair is the two
        // relations' join, whatever binds them).
        let consts = |s: &StepPlan| -> Vec<(usize, i64)> { s.args.iter().enumerate().filter_map(|(i, a)| if let Arg::Const(v) = *a { Some((i, v)) } else { None }).collect() };
        let (ac, bc) = (consts(a), consts(b));
        let cols: Vec<ColumnIndex> = ac.iter().map(|&(i, _)| ColumnIndex::new(i as u16)).collect();
        let key: Vec<i64> = ac.iter().map(|&(_, v)| v).collect(); // copy: derived (a probe's key)
        let bcols: Vec<ColumnIndex> = on.iter().map(|&(_, c)| c).chain(bc.iter().map(|&(i, _)| i)).map(|c| ColumnIndex::new(c as u16)).collect();
        let mut n = 0u64;
        self.each(a.src, a.side == Side::Before, &cols, &key, &mut |r| {
            if self.same(a, r) {
                let bkey: Vec<i64> = on.iter().map(|&(c, _)| self.get(a.src, r, c)).chain(bc.iter().map(|&(_, v)| v)).collect(); // copy: derived (a probe's key)
                self.each(b.src, b.side == Side::Before, &bcols, &bkey, &mut |_| {
                    n += 1;
                    true
                });
            }
            true
        });
        n
    }

    /// Each row of `src`'s change: gone (on the before side) −1, added +1.
    fn change(&self, src: usize, f: &mut dyn FnMut(u64, i64)) {
        match &self.srcs[src] {
            Live::Trace { before, after } => {
                let b = Bistate::new(*before, *after);
                for id in b.gone() {
                    f(rref(true, 0, id.get()), -1);
                }
                for id in b.added() {
                    f(rref(false, 0, id.get()), 1);
                }
            }
            Live::View { docs } => {
                for &doc in docs {
                    for (before, sign) in [(true, -1), (false, 1)] {
                        if let Some(t) = self.tape(src, doc, before) {
                            t.probe(&[], &[], &mut |id| {
                                f(rref(before, doc, id.get()), sign);
                                true
                            });
                        }
                    }
                }
            }
        }
    }

    /// An argument's value for tuple `refs` (`Const`, or `Eq` read through a step's row id).
    fn val(&self, t: &TermPlan, refs: &[u64], a: Arg) -> i64 {
        match a {
            Arg::Const(v) => v,
            Arg::Eq(j, c) => self.get(t.steps[j].src, refs[j], c),
            a => unreachable!("a value is a constant or a bound variable, not {a:?}"),
        }
    }

    /// The probe of step `s` for tuple `refs`: the columns its constants and bound variables fix, and their values.
    fn key(&self, t: &TermPlan, s: &StepPlan, refs: &[u64]) -> (Vec<ColumnIndex>, Vec<i64>) {
        let mut cols = Vec::new();
        let mut key = Vec::new();
        for (i, &a) in s.args.iter().enumerate() {
            if matches!(a, Arg::Const(_) | Arg::Eq(..)) {
                cols.push(ColumnIndex::new(i as u16));
                key.push(self.val(t, refs, a));
            }
        }
        (cols, key)
    }

    /// Whether row `r` of step `s` agrees with itself where a variable repeats.
    fn same(&self, s: &StepPlan, r: u64) -> bool {
        s.args.iter().enumerate().all(|(i, a)| match *a {
            Arg::Same(c) => self.get(s.src, r, i) == self.get(s.src, r, c),
            _ => true,
        })
    }

    /// Whether a row of `s`'s side `before` matches the key of the not-step's row `r` (its constants, its bound
    /// variables and its repeats, read from `r`).
    fn any_like(&self, s: &StepPlan, r: u64, before: bool) -> bool {
        let cols: Vec<ColumnIndex> = s.args.iter().enumerate().filter(|(_, a)| !matches!(a, Arg::Wild)).map(|(i, _)| ColumnIndex::new(i as u16)).collect();
        let key: Vec<i64> = cols.iter().map(|c| self.get(s.src, r, c.get())).collect();
        self.each(s.src, before, &cols, &key, &mut |_| false)
    }

    /// Every chain's start, `f(term, sign, row)`.
    fn seed(&self, f: &mut dyn FnMut(usize, i64, u64)) {
        for (ti, t) in self.plan.terms.iter().enumerate() {
            // A fact (a rule with no steps): one tuple, of no row.
            let Some(s) = t.steps.first() else {
                f(ti, t.sign, NO_ROW);
                continue;
            };
            let part_ok = |r: u64| t.parts.is_empty() || t.parts.contains(&self.get(s.src, r, 0));
            let fits = |r: u64| {
                s.args.iter().enumerate().all(|(i, a)| match *a {
                    Arg::Const(v) => self.get(s.src, r, i) == v,
                    _ => true,
                }) && self.same(s, r)
                    && part_ok(r)
            };
            match (s.op, s.side) {
                (Op::Atom, Side::Delta) => self.change(s.src, &mut |r, sign| {
                    if fits(r) {
                        f(ti, sign * t.sign, r);
                    }
                }),
                (Op::Not, Side::Delta) => {
                    // A key whose absence changed: "none after" − "none before", once a key.
                    let mut seen: BTreeSet<Vec<i64>> = BTreeSet::new();
                    self.change(s.src, &mut |r, _| {
                        if !fits(r) {
                            return;
                        }
                        let key: Vec<i64> = s.args.iter().enumerate().filter(|(_, a)| !matches!(a, Arg::Wild)).map(|(i, _)| self.get(s.src, r, i)).collect(); // copy: derived (a changed key, once)
                        if !seen.insert(key) {
                            return;
                        }
                        let sign = i64::from(!self.any_like(s, r, false)) - i64::from(!self.any_like(s, r, true));
                        if sign != 0 {
                            f(ti, sign * t.sign, r);
                        }
                    });
                }
                (Op::Atom, side) => {
                    let (cols, key) = self.key(t, s, &[]);
                    self.each(s.src, side == Side::Before, &cols, &key, &mut |r| {
                        if fits(r) {
                            f(ti, t.sign, r);
                        }
                        true
                    });
                }
                // A filter first (a `not` or a `cmp` of constants: B§7 refuses an unbound variable in either): the
                // empty tuple, kept when it passes.
                (Op::Not | Op::Cmp(_), _) => {
                    let mut probes = 0;
                    self.step(ti, 0, &[NO_ROW], &mut probes, &mut |r| {
                        debug_assert!(r.is_none(), "a filter binds no row");
                        f(ti, t.sign, NO_ROW);
                    });
                }
            }
        }
    }

    /// Tuple `refs` of chain `ti` through step `d`, each result `f(row)` (`None`: kept as it is, for a filter).
    fn step(&self, ti: usize, d: usize, refs: &[u64], probes: &mut u64, f: &mut dyn FnMut(Option<u64>)) {
        let t = &self.plan.terms[ti];
        let s = &t.steps[d];
        match s.op {
            Op::Cmp(c) => {
                if c.test(self.val(t, refs, s.args[0]), self.val(t, refs, s.args[1])) {
                    f(None);
                }
            }
            Op::Atom => {
                let (cols, key) = self.key(t, s, refs);
                *probes += 1;
                self.each(s.src, s.side == Side::Before, &cols, &key, &mut |r| {
                    if self.same(s, r) {
                        f(Some(r));
                    }
                    true
                });
            }
            Op::Not => {
                let (cols, key) = self.key(t, s, refs);
                *probes += 1;
                let found = self.each(s.src, s.side == Side::Before, &cols, &key, &mut |r| !self.same(s, r));
                if !found {
                    f(None);
                }
            }
        }
    }
}

// Frames: words at the body, `[n, stride]`, then `n` tuples of `stride` words: the chain, the sign, a row id a step.
const F_N: usize = BODY_AT;
const F_STRIDE: usize = BODY_AT + 8;
const F_AT: usize = BODY_AT + 16;

fn frame_word(b: &[u8], i: usize, k: usize) -> i64 {
    let stride = word::<i64>(b, F_STRIDE) as usize;
    word::<i64>(b, F_AT + 8 * (i * stride + k))
}

/// The tuples of a frames block: `(chain, sign, row ids)`, read where they lie.
pub(crate) fn tuples(b: &[u8]) -> impl Iterator<Item = (usize, i64, Vec<u64>)> + '_ {
    let (n, stride) = (word::<i64>(b, F_N) as usize, word::<i64>(b, F_STRIDE) as usize);
    // copy: derived (a tuple's few row ids, as a step reads them)
    (0..n).map(move |i| (frame_word(b, i, 0) as usize, frame_word(b, i, 1), (2..stride).map(|k| frame_word(b, i, k) as u64).collect()))
}

/// A frames block's body bytes for `n` tuples of a plan `width` steps wide.
pub(crate) fn frames_bytes(n: usize, width: usize) -> usize {
    16 + 8 * n * (width + 2)
}

/// What makes a frames block's tuples: told whether it places them (else counts), it gives each `(term, sign, refs)`.
type Produce<'a> = dyn FnMut(bool, &mut dyn FnMut(usize, i64, &[u64])) + 'a;

/// An emit into a frames block `out`: counted, then placed (`produce` runs twice, told which); past its room, the
/// room it needs named in control `ctl`.
fn emit_frames(out: &mut [u8], ctl: &mut [u8], width: usize, produce: &mut Produce<'_>) {
    let mut n = 0usize;
    produce(false, &mut |_, _, _| n += 1);
    let stride = width + 2;
    let room = (out.len() - F_AT) / (8 * stride);
    if n > room {
        set_word::<i64>(ctl, C_NEEDED, n as i64);
        return;
    }
    set_word::<i64>(out, F_N, n as i64);
    set_word::<i64>(out, F_STRIDE, stride as i64);
    let mut i = 0;
    produce(true, &mut |ti, sign, refs| {
        let at = F_AT + 8 * i * stride;
        set_word::<i64>(out, at, ti as i64);
        set_word::<i64>(out, at + 8, sign);
        for k in 0..width {
            set_word::<i64>(out, at + 16 + 8 * k, refs.get(k).map_or(0, |&r| r as i64));
        }
        i += 1;
    });
    set_word::<i64>(ctl, C_ROWS, n as i64);
}

// The slots every one of the three takes, and their control words.
const PLAN: usize = 0;
const ATOMS: usize = 1;
const OUT: usize = 2;
const CONTROL: usize = 3;
const FRAMES: usize = 4;
const C_ARG: usize = BODY_AT;
const C_NEEDED: usize = BODY_AT + 8;
const C_PROBES: usize = BODY_AT + 16;
const C_ROWS: usize = BODY_AT + 24;
const C_TERM: usize = BODY_AT + 32;
const C_LIMIT: usize = BODY_AT + 40;
const CONTROL_BYTES: usize = 48;

fn seed(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([plan, atoms], [out, ctl]) = c.views([PLAN, ATOMS], [OUT, CONTROL]);
    let names = Names::new(atoms);
    let env = Env::new(table, plan, &names);
    let width = env.plan.width();
    emit_frames(out, ctl, width, &mut |_, f| env.seed(&mut |ti, sign, r| f(ti, sign, &[r])));
}

fn step(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([plan, atoms, frames], [out, ctl]) = c.views([PLAN, ATOMS, FRAMES], [OUT, CONTROL]);
    let names = Names::new(atoms);
    let env = Env::new(table, plan, &names);
    let d = word::<i64>(ctl, C_ARG) as usize;
    // A trial's launch (0207): only chain `C_TERM − 1`'s tuples, at most `C_LIMIT` of them (0: every chain, all).
    let (only, limit) = (word::<i64>(ctl, C_TERM), word::<i64>(ctl, C_LIMIT));
    let width = env.plan.width();
    let mut probes = 0u64;
    emit_frames(out, ctl, width, &mut |placing, f| {
        let mut taken = 0i64;
        for (ti, sign, refs) in tuples(frames) {
            if only > 0 && ti as i64 != only - 1 {
                continue;
            }
            if limit > 0 {
                if taken == limit {
                    break;
                }
                taken += 1;
            }
            if d >= env.plan.terms[ti].steps.len() {
                f(ti, sign, &refs);
                continue;
            }
            let mut buf = refs.clone(); // copy: derived (a tuple's few row ids, extended by one)
            let mut p = 0;
            env.step(ti, d, &refs, &mut p, &mut |r| {
                buf[d] = r.unwrap_or(refs[d]);
                f(ti, sign, &buf);
            });
            if !placing {
                probes += p;
            }
        }
    });
    set_word::<i64>(ctl, C_PROBES, probes as i64);
}

fn head(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([plan, atoms, frames, ctl], [runs]) = c.views([PLAN, ATOMS, FRAMES, CONTROL], [OUT]);
    let h = word::<i64>(ctl, C_ARG) as usize;
    let names = Names::new(atoms);
    let env = Env::new(table, plan, &names);
    let mine = |ti: usize| env.plan.terms[ti].head == h;
    let n = tuples(frames).filter(|t| mine(t.0)).count();
    let Some(mut at) = runs_claim_in(runs, n as u32) else { return };
    let mut row = Vec::new();
    for (ti, sign, refs) in tuples(frames).filter(|t| mine(t.0)) {
        let t = &env.plan.terms[ti];
        row.clear();
        row.extend(t.out.iter().map(|&a| env.val(t, &refs, a)));
        if let Some(l) = &t.level {
            row.push(l.iter().map(|&(j, c)| 1 + env.val(t, &refs, Arg::Eq(j, c))).max().unwrap_or(0));
        }
        runs_set_in(runs, at, &row, sign as i32);
        at += 1;
    }
}

fn one(_: &Ctx<'_, '_, '_>) -> u32 {
    1
}

static EMIT: [Phase; 1] = [Phase { name: "emit", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByScan, bounds: &[Bound::Capacity(Over::Refuse)] }];

const fn slots(out: &'static str, out_kind: u32) -> [SlotSpec; 5] {
    [
        SlotSpec { name: "plan", mode: Mode::Read, kind: kind::CONTROL },
        SlotSpec { name: "atoms", mode: Mode::Read, kind: kind::ATOMS },
        SlotSpec { name: out, mode: Mode::Write, kind: out_kind },
        SlotSpec { name: "control", mode: Mode::ReadWrite, kind: kind::CONTROL },
        SlotSpec { name: "frames", mode: Mode::Read, kind: kind::SCRATCH },
    ]
}

static SEED_SLOTS: [SlotSpec; 4] = {
    let s = slots("frames out", kind::SCRATCH);
    [s[0], s[1], s[2], s[3]]
};
static STEP_SLOTS: [SlotSpec; 5] = slots("frames out", kind::SCRATCH);
static HEAD_SLOTS: [SlotSpec; 5] = slots("runs", kind::SCRATCH);

/// A frames slot's facts, by meaning: its tuples, sorted (their order is the launch's); control's room needed.
fn frames_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == CONTROL {
        return vec![vec![word::<i64>(bytes, C_NEEDED)]];
    }
    let mut rows: Vec<Vec<i64>> = tuples(bytes).map(|(t, s, r)| [t as i64, s].into_iter().chain(r.into_iter().map(|x| x as i64)).collect()).collect(); // copy: derived (a check's reading of facts)
    rows.sort();
    rows
}

/// A head's runs slot's facts, by meaning: its `(row, diff)`s, sorted.
fn runs_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == CONTROL {
        return Vec::new();
    }
    let r = Runs::new(bytes);
    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..r.head.shape.arity()).map(|c| r.get(i, c)).chain([i64::from(r.diff(i))]).collect()).collect(); // copy: derived (a check's reading of facts)
    rows.sort();
    rows
}

/// `seed`: read the plan (its sources by reference), atoms · write frames · rw control.
pub static SEED: Signature = Signature::new("seed", &SEED_SLOTS, ParamsLayout::of::<NoParams>(), &EMIT, Independence::Along("chains: a chain's start is its own"), frames_facts);
/// `step`: read the plan, atoms, frames · write frames · rw control (the depth in).
pub static STEP: Signature = Signature::new("step", &STEP_SLOTS, ParamsLayout::of::<NoParams>(), &EMIT, Independence::Along("tuples: a tuple's matches are its own"), frames_facts);
/// `head`: read the plan, atoms, frames · write a head's δ runs · rw control (the head in).
pub static HEAD: Signature = Signature::new("head", &HEAD_SLOTS, ParamsLayout::of::<NoParams>(), &EMIT, Independence::Along("tuples: a tuple's head row is its own"), runs_facts);

static SEED_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: seed, again: None }];
static STEP_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: step, again: None }];
static HEAD_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: head, again: None }];

/// The three operations, declared, their host versions their references.
#[derive(Clone, Copy)]
pub struct BodyOps {
    pub seed: OpId,
    pub step: OpId,
    pub head: OpId,
}

impl BodyOps {
    pub fn declare(reg: &mut Registry) -> Result<BodyOps, String> {
        let seed = reg.declare(&SEED)?;
        reg.register_reference(Version::host(seed, "seed on the host", &SEED_HOST, &EMIT))?;
        let step = reg.declare(&STEP)?;
        reg.register_reference(Version::host(step, "step on the host", &STEP_HOST, &EMIT))?;
        let head = reg.declare(&HEAD)?;
        reg.register_reference(Version::host(head, "head on the host", &HEAD_HOST, &EMIT))?;
        Ok(BodyOps { seed, step, head })
    }
}

/// The plan block: its words, and a reference to every block it reads; frozen.
pub(crate) fn plan_block(dir: &mut Directory, plan: &Plan) -> Result<BlockId, String> {
    plan_block_keyed(dir, plan, &[])
}

/// The plan block with each chain's line key after the plan's words (`plan::lines`; the body's operations read the
/// plan's words alone).
pub(crate) fn plan_block_keyed(dir: &mut Directory, plan: &Plan, keys: &[i64]) -> Result<BlockId, String> {
    let words: Vec<i64> = plan.words().into_iter().chain(std::iter::once(keys.len() as i64)).chain(keys.iter().copied()).collect();
    let id = dir.make(kind::CONTROL, 8 + 8 * words.len());
    {
        let b = dir.block(id).expect("just made");
        let mut w = pin_block(&b, "a plan", true)?;
        let bytes = w.bytes_mut().expect("a write pin");
        set_word::<i64>(bytes, BODY_AT, words.len() as i64);
        for (i, &x) in words.iter().enumerate() {
            set_word::<i64>(bytes, BODY_AT + 8 + 8 * i, x);
        }
    }
    for (b, at) in plan.reads() {
        dir.refer(id, b, at)?;
    }
    dir.freeze(id)?;
    Ok(id)
}

fn control_with(dir: &mut Directory, arg: i64) -> Result<BlockId, String> {
    control_for(dir, arg, None)
}

/// A control block: its argument, and for a trial's launch, the one chain it runs and its tuples' limit.
fn control_for(dir: &mut Directory, arg: i64, only: Option<(usize, usize)>) -> Result<BlockId, String> {
    let id = dir.make(kind::CONTROL, CONTROL_BYTES);
    let b = dir.block(id).expect("just made");
    let mut w = pin_block(&b, "a body's control", true)?;
    let bytes = w.bytes_mut().expect("a write pin");
    set_word::<i64>(bytes, C_ARG, arg);
    if let Some((ti, limit)) = only {
        set_word::<i64>(bytes, C_TERM, ti as i64 + 1);
        set_word::<i64>(bytes, C_LIMIT, limit as i64);
    }
    Ok(id)
}

fn control_word(dir: &Directory, id: BlockId, at: usize) -> Result<i64, String> {
    Ok(word::<i64>(dir.read("a body's control", &[id])?.block(0), at))
}

/// Runs `op` into a new frames block from `frames` (`None`: `seed`), again at the room it names when it outgrew the
/// first: the new block, and the probes it made.
#[allow(clippy::too_many_arguments)]
fn emit(reg: &Registry, op: OpId, dir: &mut Directory, plan: BlockId, atoms: BlockId, frames: Option<BlockId>, arg: i64, width: usize, room: &mut usize) -> Result<(BlockId, u64), String> {
    emit_only(reg, op, dir, plan, atoms, frames, arg, width, room, None)
}

/// `emit`, for a trial's launch: only chain `only.0`'s first `only.1` tuples.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_only(reg: &Registry, op: OpId, dir: &mut Directory, plan: BlockId, atoms: BlockId, frames: Option<BlockId>, arg: i64, width: usize, room: &mut usize, only: Option<(usize, usize)>) -> Result<(BlockId, u64), String> {
    loop {
        let out = dir.make(kind::SCRATCH, frames_bytes(*room, width));
        let ctl = control_for(dir, arg, only)?;
        let slots: Vec<BlockId> = [plan, atoms, out, ctl].into_iter().chain(frames).collect();
        let ran = run_cpu(reg, dir, op, &slots, &[], Tiling::WHOLE);
        let (needed, probes) = match ran.and_then(|()| Ok((control_word(dir, ctl, C_NEEDED)?, control_word(dir, ctl, C_PROBES)?))) {
            Ok(x) => x,
            Err(e) => {
                dir.release(out);
                dir.release(ctl);
                return Err(e);
            }
        };
        dir.release(ctl);
        if needed > 0 {
            dir.release(out);
            *room = needed as usize + needed as usize / 4;
            continue;
        }
        return Ok((out, probes as u64));
    }
}

/// Runs `plan`'s chains: `seed`, a `step` a depth, then `head` a head into its runs block `runs[h]` (a head no chain
/// writes is not launched). The probes the steps made. A head's runs past their room are the caller's to open
/// again (`Runs::needed`).
pub fn run_plan(reg: &Registry, ops: &BodyOps, dir: &mut Directory, atoms: BlockId, plan: &Plan, runs: &[BlockId]) -> Result<u64, String> {
    run_plan_with(reg, ops, dir, atoms, plan, runs, &mut |_, _, _, pb| Ok(pb))
}

/// What runs between a plan's depths (`plan::Round::check`): told the depth just run, its frames and the plan block,
/// it gives the plan block the next depths read (the same, or a handoff's).
pub(crate) type Between<'a> = dyn FnMut(&mut Directory, usize, BlockId, BlockId) -> Result<BlockId, String> + 'a;

/// `run_plan`, with `between` after `seed` and after each depth's `step`; the plan block it gives is let go by the
/// caller's, never here, unless it is the first, which this makes.
pub(crate) fn run_plan_with(reg: &Registry, ops: &BodyOps, dir: &mut Directory, atoms: BlockId, plan: &Plan, runs: &[BlockId], between: &mut Between<'_>) -> Result<u64, String> {
    let pb = plan_block(dir, plan)?;
    let ran = run_block(reg, ops, dir, atoms, plan, pb, runs, between);
    dir.release(pb);
    ran
}

/// The chains of plan block `pb` (`plan`'s words, or a plan of the same chains' lengths and heads), run as
/// `run_plan` runs them, `between` after each depth.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_block(reg: &Registry, ops: &BodyOps, dir: &mut Directory, atoms: BlockId, plan: &Plan, pb: BlockId, runs: &[BlockId], between: &mut Between<'_>) -> Result<u64, String> {
    if plan.terms.is_empty() {
        return Ok(0);
    }
    let width = plan.width();
    let mut room = 256usize;
    let (mut frames, mut probes) = emit(reg, ops.seed, dir, pb, atoms, None, 0, width, &mut room)?;
    let mut pb = pb;
    let ran = (|| {
        pb = between(dir, 0, frames, pb)?;
        for d in 1..width {
            let next = emit(reg, ops.step, dir, pb, atoms, Some(frames), d as i64, width, &mut room);
            dir.release(frames);
            let (f, p) = next?;
            frames = f;
            probes += p;
            pb = between(dir, d, frames, pb)?;
        }
        let heads: BTreeSet<usize> = plan.terms.iter().map(|t| t.head).collect();
        for h in heads {
            let ctl = control_with(dir, h as i64)?;
            let out = run_cpu(reg, dir, ops.head, &[pb, atoms, runs[h], ctl, frames], &[], Tiling::WHOLE);
            dir.release(ctl);
            out?;
        }
        Ok(probes)
    })();
    dir.release(frames);
    ran
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
