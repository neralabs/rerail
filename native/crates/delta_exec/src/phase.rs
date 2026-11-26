//! Δwave, Δfix and Δphase (decision 0686; B§3 sentences 4 and 5; P§3, P§5; *0686: Operations* §10: "Δwave is
//! cache_open → seed → steps → head → consolidate → tally → merge → cache_commit for a stratum …; Δfix that in a loop
//! on tally's flag; Δphase the strata in k_order").
//!
//! A **stratum** (`Stratum`) is a module's rules, a delta operator: its input ports are the document set it reads
//! views of (when it reads any) and the relations its rules read; its global is the atoms; its outputs are its heads'
//! traces: a non-recursive stratum's each with its derivation counts beside it (0678 step 9), a recursive one's each
//! with its level as its last column (0364 §1: "a level per row, as a column of its relations"; a row stays only on a
//! derivation from rows of lower level). A level is a witness, carried from edit to edit, so the laws compare a
//! levelled output by its key (`contract::harness_on`). Its rules are those of the program on each side: an unchanged
//! module's are the same, and a changed one's are its old and its new (O3: the module is the grain, its rules removed
//! and its new ones run cold). A head's shape is its rules' (its kind of grain, the stratum's recursion): an edit that
//! changes it runs the level again from ∅, since a trace keeps one shape.
//!
//! - **Δwave** (a stratum that reads no head of its own): its single-file rules at the file's grain, its cross-file
//!   rules by their delta terms (O1: a term a changed step, the steps before it on the after side and after it on the
//!   before side). A changed document's derivations are retracted from the counts kept for it, read where they lie
//!   (`drop::drop_counted`: "the file's last result is its output trace's before", P§3), and its rules run cold over
//!   its new tape; a removed rule's derivations run cold on the before side, −1, and an added one's on the after side.
//!   `tally` nets each head's derivations against its counts.
//! - **Δfix** (a stratum that reads its own heads): rounds of its rules over working layers of its heads
//!   (`Trace::over`), until a round adds nothing. Removals by levels (P§5, 0364): a row that lost a derivation is
//!   checked, level by level from the lowest, for a derivation from rows of lower level that remain; one with none
//!   goes, and the rows derived from it are checked in turn; a row gone that is still derived comes back, above every
//!   level, and the rounds go on from what was added.
//!
//! A **level** (`Level`) is one phase of B§8's loop: a program, its facts in, its documents (the `load` answers its
//! facts ask, through the document set), and its strata, run in `k_order` (Δphase). Its heads are traces it keeps
//! across runs, a relation each; a stratum's run reads each relation between where the level's last run left it and
//! where it is now (L6: the composed operator's delta). What a level exports (O6): its K-form relations are the next
//! level's program, the rest its facts in.

use crate::body::{Arg, BodyOps, Decls, DocAt, Op, Plan, Side, Src, StepPlan, TermPlan, Text};
use crate::plan::{PlanOps, Round};
use crate::boot::DocSet;
use crate::contract::{Asks, At, Bench, CacheW, Cx, DeltaOp, GlobalIn, In, Instance, Out, Port, Ports, PortIn, PortOut, Programmed, Work, rows_at};
use crate::dir::{BlockId, Directory};
use crate::drop::{DropOps, drop_counted};
use crate::files::LeavesW;
use crate::kform::{self, RELATIONS, Refused};
use crate::l1::{row_blobs, tape_shape};
use crate::program::{Cmp, Kind, Known, Program, Rule, Term};
use crate::shapes::{Facts, Shapes};
use crate::sym::Atoms;
use crate::tally::{self, TallyOps};
use crate::trace::{RollUp, Trace, open_runs};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{ColType, Runs, Shape};
use rerail_blocks::relation::View;
use std::collections::{BTreeMap, BTreeSet};

/// The operations a stratum runs.
#[derive(Clone, Copy)]
pub struct PhaseOps {
    pub body: BodyOps,
    pub tally: TallyOps,
    pub drops: DropOps,
    pub plan: PlanOps,
}

impl PhaseOps {
    pub fn declare(reg: &mut crate::op::Registry, drops: DropOps) -> Result<PhaseOps, String> {
        Ok(PhaseOps { body: BodyOps::declare(reg)?, tally: TallyOps::declare(reg)?, drops, plan: PlanOps::declare(reg)? })
    }
}

/// A head's trace's shape: a K-form relation's own (`kform::shape`), else 64-bit columns sorted in order,
/// partitioned by the document when its rules are single-file (its first column, 32 bits).
pub fn head_shape(name: &str, arity: usize, single_file: bool) -> Shape {
    if let Some(i) = RELATIONS.iter().position(|r| r.0 == name && r.1 == arity) {
        return kform::shape(i);
    }
    let columns = (0..arity).map(|c| if c == 0 && single_file { ColType::I32 } else { ColType::I64 }).collect();
    Shape::run(columns, (0..arity as u16).collect(), single_file.then_some(0))
}

/// A head's derivation counts' shape (0678 step 9): its columns and a count, 64 bits.
pub fn counts_shape(head: &Shape) -> Shape {
    let mut columns = head.columns.clone();
    columns.push(ColType::I64);
    let mut order = head.order.clone();
    order.push(columns.len() as u16 - 1);
    Shape::run(columns, order, head.partition)
}

/// A recursive head's shape (0364 §1: "a level per row, as a column of its relations"): its columns and its level,
/// the last, 32 bits.
pub fn levelled(head: &Shape) -> Shape {
    let mut s = head.clone();
    s.columns.push(ColType::I32);
    s.order.push(s.columns.len() as u16 - 1);
    s.levels = true;
    s
}

/// A head's rows apart from any level: its columns, in its order.
fn key_shape(head: &Shape) -> Shape {
    let mut s = head.clone();
    if s.levels {
        s.columns.pop();
        s.order.pop();
        s.levels = false;
    }
    s
}

/// A head of a stratum: its relation, its arity, its trace's shape (levelled in a recursive stratum), and its
/// derivation counts' shape (a non-recursive stratum's).
#[derive(Clone, Debug)]
pub struct Head {
    pub rel: i64,
    pub arity: usize,
    pub shape: Shape,
    pub counts: Option<Shape>,
}

/// A stratum: a module's rules on each side, the relations they read and the heads they write.
pub struct Stratum {
    pub ops: PhaseOps,
    pub module: i64,
    pub before: Vec<Rule>,
    pub after: Vec<Rule>,
    pub known: Known,
    /// Whether it reads views: its input port 0 is then the document set (`l1::tape_shape`).
    pub tape: bool,
    /// The relations it reads, a port each after the document set's.
    pub ins: Vec<(i64, Shape)>,
    pub heads: Vec<Head>,
    /// Whether its rules read its own heads (Δfix).
    pub recursive: bool,
    /// The facts earlier strata's rules give the relations it reads (0361, `shapes::infer`).
    pub inferred: Vec<(i64, Facts)>,
}

/// What a stratum's run reads, on its two sides: each input port's heads and the partitions it changed in, the
/// documents, and the atoms.
struct Sides {
    heads: Vec<(Option<BlockId>, Option<BlockId>)>,
    changed: Vec<bool>,
    parts: Vec<BTreeSet<i64>>,
    docs: Vec<DocAt>,
    atoms: BlockId,
    /// The planner's memo: the operator's cache block, open for writing (`None`: the plain function's run, which
    /// plans from an empty one).
    memo: Option<BlockId>,
}

fn cell(v: &View<'_>, id: RowId, c: usize) -> i64 {
    Read::get(v, id, ColumnIndex::new(c as u16))
}

fn live<'v>(v: &'v View<'_>) -> impl Iterator<Item = RowId> + 'v {
    (0..Read::end(v).get()).map(RowId::new).filter(|&id| Read::live(v, id))
}

/// The documents of a document set's two states, each with its text on each side, where the blobs place them.
fn docs_of(cx: &Cx<'_>, before: Option<&View<'_>>, after: &View<'_>) -> Result<Vec<DocAt>, String> {
    let text = |v: &View<'_>, id: RowId| -> Result<Text, String> {
        let row: Vec<i64> = (0..Read::arity(v)).map(|c| cell(v, id, c)).collect(); // copy: derived (a document's row: its blobs' names)
        let (ids, syms) = row_blobs(&row);
        let get = |b| cx.blobs.get(cx.dir, &b)?.ok_or_else(|| format!("document {}: a blob the document set names, which nothing holds", row[0]));
        Ok(Text { src: get(ids.source)?, tape: get(ids.tape)?, syms: get(syms)?.block })
    };
    let mut docs: BTreeMap<i64, DocAt> = BTreeMap::new();
    if let Some(b) = before {
        for id in live(b) {
            let d = cell(b, id, 0);
            docs.insert(d, DocAt { d, before: Some(text(b, id)?), after: None });
        }
    }
    for id in live(after) {
        let d = cell(after, id, 0);
        let t = Some(text(after, id)?);
        docs.entry(d).or_insert(DocAt { d, before: None, after: None }).after = t;
    }
    Ok(docs.into_values().collect())
}

/// How a chain walks a rule's steps.
#[derive(Clone, Copy)]
struct Walk<'r> {
    rule: &'r Rule,
    /// The rule's steps in the order the chain runs them, each with its side.
    order: &'r [(usize, Side)],
    head: usize,
    sign: i64,
    /// A first step over candidate rows of the head (a recheck): its source.
    cand: Option<usize>,
    /// Each step over an own head followed by a filter keeping rows of level below this.
    below: Option<i64>,
}

/// A stratum's plan, made a chain at a time.
struct Planner<'s> {
    st: &'s Stratum,
    s: &'s Sides,
    plan: Plan,
    /// Each chain's line key (`plan`'s lines): its rule's content, the order and sides its steps are written in, its
    /// head, its sign, and whether it starts from candidates or keeps levels below a bound.
    keys: Vec<i64>,
    /// Each own head's working state: its heads before and after (a recursive stratum's).
    own: Vec<(Option<BlockId>, Option<BlockId>)>,
}

/// Which documents a view step reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Docs<'a> {
    All,
    Changed,
    Of(&'a BTreeSet<i64>),
}

impl<'s> Planner<'s> {
    fn new(st: &'s Stratum, s: &'s Sides) -> Planner<'s> {
        let docs = s.docs.clone(); // copy: derived (the documents' places, the plan's)
        let mut p = Planner { st, s, plan: Plan { srcs: Vec::new(), docs, decls: Decls::default(), terms: Vec::new() }, keys: Vec::new(), own: Vec::new() };
        // The declarations (0361): their relations as sources, read where they lie by the planning operations.
        p.plan.decls.inferred = st.inferred.clone(); // copy: derived (the facts inferred for its reads, its plan's words)
        p.plan.decls.one = st.known.one.unwrap_or(0);
        p.plan.decls.few = st.known.few.unwrap_or(0);
        if let Some(k) = st.known.key.filter(|&k| st.port_of(k).is_some()) {
            p.plan.decls.key = Some(p.source(k, 3, Docs::All));
        }
        if let Some(f) = st.known.fanout.filter(|&f| st.port_of(f).is_some()) {
            p.plan.decls.fanout = Some(p.source(f, 3, Docs::All));
        }
        p
    }

    /// Source `src` of relation `rel`: one a relation, kept once.
    fn src(&mut self, src: Src, rel: i64) -> usize {
        let d = &mut self.plan.decls;
        if let Some(i) = self.plan.srcs.iter().zip(&d.rels).position(|(s, r)| *s == src && *r == rel) {
            return i;
        }
        self.plan.srcs.push(src);
        d.rels.push(rel);
        self.plan.srcs.len() - 1
    }

    /// The source of a step reading relation `rel` (`args` of them), its views over `docs`.
    fn source(&mut self, rel: i64, arity: usize, docs: Docs<'_>) -> usize {
        if let Some(view) = self.st.known.view(rel) {
            let places = self
                .s
                .docs
                .iter()
                .enumerate()
                .filter(|(_, d)| match docs {
                    Docs::All => true,
                    Docs::Changed => d.before != d.after,
                    Docs::Of(set) => set.contains(&d.d),
                })
                .map(|(i, _)| i)
                .collect();
            return self.src(Src::View { view, docs: places }, rel);
        }
        if let Some(k) = self.st.heads.iter().position(|h| h.rel == rel)
            && self.st.recursive
        {
            let (before, after) = self.own[k];
            return self.src(Src::Trace { arity: self.st.heads[k].arity + 1, before, after }, rel);
        }
        match self.st.port_of(rel) {
            Some(p) => {
                let (before, after) = self.s.heads[p];
                // The trace's own columns: a recursive relation's carry its level after the atom's.
                let arity = self.st.ins[p - usize::from(self.st.tape)].1.columns.len();
                self.src(Src::Trace { arity, before, after }, rel)
            }
            None => self.src(Src::Trace { arity, before: None, after: None }, rel),
        }
    }

    /// A chain of rule `c.rule`, its views over `docs`, its first steps restricted to `parts`.
    fn chain(&mut self, c: Walk<'_>, docs: Docs<'_>, changed: Docs<'_>, parts: &[i64]) {
        let mut steps = Vec::new();
        let mut bound: BTreeMap<i64, (usize, usize)> = BTreeMap::new();
        let bind = |args: &[Term], j: usize, bound: &mut BTreeMap<i64, (usize, usize)>| -> Vec<Arg> {
            let mut here: BTreeMap<i64, usize> = BTreeMap::new();
            let out = args
                .iter()
                .enumerate()
                .map(|(i, t)| match *t {
                    Term::Wild => Arg::Wild,
                    Term::Const(v) => Arg::Const(v),
                    Term::Var(v) => match (bound.get(&v), here.get(&v)) {
                        (Some(&(bj, bc)), _) => Arg::Eq(bj, bc),
                        (None, Some(&c0)) => Arg::Same(c0),
                        (None, None) => {
                            here.insert(v, i);
                            Arg::Bind
                        }
                    },
                })
                .collect();
            for (v, i) in here {
                bound.insert(v, (j, i));
            }
            out
        };
        if let Some(src) = c.cand {
            let args = bind(&c.rule.args, 0, &mut bound);
            steps.push(StepPlan { op: Op::Atom, src, side: Side::After, args });
        }
        // The levels the derivation reads: each own head's row's (0364 §2.4).
        let mut levels = Vec::new();
        for &(si, side) in c.order {
            let st = &c.rule.steps[si];
            let j = steps.len();
            match st.kind {
                Kind::Cmp(cmp) => {
                    let args = st.args.iter().map(|t| match *t {
                        Term::Var(v) => {
                            let (bj, bc) = bound[&v];
                            Arg::Eq(bj, bc)
                        }
                        Term::Const(v) => Arg::Const(v),
                        Term::Wild => Arg::Wild,
                    });
                    steps.push(StepPlan { op: Op::Cmp(cmp), src: 0, side, args: args.collect() });
                }
                Kind::Atom | Kind::Not => {
                    let d = if side == Side::Delta { changed } else { docs };
                    let src = self.source(st.rel, st.args.len(), d);
                    let args = bind(&st.args, j, &mut bound);
                    let op = if st.kind == Kind::Atom { Op::Atom } else { Op::Not };
                    steps.push(StepPlan { op, src, side, args });
                    let own = self.st.recursive.then(|| self.st.heads.iter().position(|h| h.rel == st.rel)).flatten();
                    if let (Some(k), Op::Atom) = (own, op) {
                        levels.push((j, self.st.heads[k].arity));
                        if let Some(below) = c.below {
                            let level = Arg::Eq(j, self.st.heads[k].arity);
                            steps.push(StepPlan { op: Op::Cmp(Cmp::Lt), src: 0, side: Side::After, args: vec![level, Arg::Const(below)] });
                        }
                    }
                }
            }
        }
        let out = c
            .rule
            .args
            .iter()
            .map(|t| match *t {
                Term::Var(v) => {
                    let (j, col) = bound[&v];
                    Arg::Eq(j, col)
                }
                Term::Const(v) => Arg::Const(v),
                Term::Wild => unreachable!("a head argument is never _ (refused)"),
            })
            .collect();
        let level = self.st.recursive.then_some(levels);
        self.plan.terms.push(TermPlan { head: c.head, sign: c.sign, parts: parts.to_vec(), steps, out, level });
        self.keys.push(line_key(&c));
    }
}

/// A chain's line key: a fixed hash (FNV-1a, the same in every build and run) of what makes it the same line from
/// round to round and run to run.
fn line_key(c: &Walk<'_>) -> i64 {
    use std::hash::{Hash, Hasher};
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for &b in bytes {
                self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    (c.rule.head, &c.rule.args, &c.rule.steps).hash(&mut h);
    for &(si, side) in c.order {
        (si, side as u8).hash(&mut h);
    }
    (c.head, c.sign, c.cand.is_some(), c.below).hash(&mut h);
    h.finish() as i64
}

/// A rule's steps in their own order, each on `side`.
fn cold(rule: &Rule, side: Side) -> Vec<(usize, Side)> {
    (0..rule.steps.len()).map(|i| (i, side)).collect()
}

/// A rule's delta term at step `i`: `i` first on its change, then the others in order, those before it on the after
/// side and those after it on the before side (O1; 0678 step 5, for k steps).
fn term_at(rule: &Rule, i: usize) -> Vec<(usize, Side)> {
    std::iter::once((i, Side::Delta)).chain((0..rule.steps.len()).filter(|&j| j != i).map(|j| (j, if j < i { Side::After } else { Side::Before }))).collect()
}

/// A run's derivations, a head each: `(row, diff)` pairs read from their runs.
type Derived = Vec<Vec<(Vec<i64>, i32)>>;

/// A recursive stratum's rows found, a head each: each key's level.
type Found = Vec<BTreeMap<Vec<i64>, i64>>;

/// Each derivation added in `derived` (a row and its level last) into `into`, by its key: the lowest level any of its
/// derivations gives.
fn found(into: &mut Found, derived: Derived) {
    for (k, rows) in derived.into_iter().enumerate() {
        for (mut r, d) in rows {
            if d > 0 {
                let l = r.pop().expect("a level");
                let e = into[k].entry(r).or_insert(l);
                *e = (*e).min(l);
            }
        }
    }
}

impl Stratum {
    /// The stratum of module `module`, its rules `before` and `after`, reading relations of shapes `shapes` (those it
    /// does not find there are read as ∅), its heads' names from `atoms`. Refused when one head is written by
    /// single-file and cross-file rules both (sentence 5 splits a stratum's rules by their reads, and a head is
    /// retracted at one grain).
    pub fn new(ops: PhaseOps, module: i64, before: Vec<Rule>, after: Vec<Rule>, known: Known, shapes: &BTreeMap<i64, Shape>, atoms: &[u8]) -> Result<Stratum, Refused> {
        let (heads, recursive) = Stratum::heads_of(module, &before, &after, &known, atoms)?;
        let all: Vec<&Rule> = before.iter().chain(&after).collect();
        // A view is read with its own columns.
        for r in &all {
            for st in r.steps.iter().filter(|s| !matches!(s.kind, Kind::Cmp(_))) {
                if let Some(v) = known.view(st.rel)
                    && v.arity() != st.args.len()
                {
                    return Err(Refused { module, rule: r.id, why: format!("step {:?} reads view {} of {} columns with {}", st.key, crate::sym::name_of(atoms, st.rel), v.arity(), st.args.len()) });
                }
            }
        }
        let own: BTreeSet<i64> = heads.iter().map(|h| h.rel).collect();
        let tape = all.iter().any(|r| r.reads().any(|(_, rel)| known.view(rel).is_some()));
        let mut ins: Vec<(i64, Shape)> = Vec::new();
        for r in &all {
            for s in r.steps.iter().filter(|s| !matches!(s.kind, Kind::Cmp(_))) {
                if known.view(s.rel).is_some() || own.contains(&s.rel) || ins.iter().any(|i| i.0 == s.rel) {
                    continue;
                }
                if let Some(sh) = shapes.get(&s.rel) {
                    ins.push((s.rel, sh.clone()));
                }
            }
        }
        // The shapes its relations declare (0361), read by its planner: the declarations of earlier strata and the
        // facts in.
        for d in [known.key, known.fanout].into_iter().flatten() {
            if !own.contains(&d) && !ins.iter().any(|i| i.0 == d)
                && let Some(sh) = shapes.get(&d)
            {
                ins.push((d, sh.clone()));
            }
        }
        Ok(Stratum { ops, module, before, after, known, tape, ins, heads, recursive, inferred: Vec::new() })
    }

    /// It, with the facts `inferred` gives the relations it reads and does not write (0361).
    pub fn with_inferred(mut self, inferred: &Shapes) -> Stratum {
        let own: BTreeSet<i64> = self.heads.iter().map(|h| h.rel).collect();
        let reads: BTreeSet<i64> = self.before.iter().chain(&self.after).flat_map(|r| r.reads().map(|(_, rel)| rel)).filter(|r| !own.contains(r)).collect();
        self.inferred = reads.into_iter().filter_map(|r| inferred.get(&r).map(|f| (r, f.clone()))).collect(); // copy: derived (the few facts inferred for what it reads)
        self
    }

    /// The heads module `module` writes with rules `before` and `after`, and whether it is recursive: each head's
    /// shape its rules' on the side that has them (the after side, else the before side). Refused when a head is
    /// written by single-file and cross-file rules both (sentence 5 splits a stratum's rules by their reads, and a
    /// head is retracted at one grain), and when a nullary head is recursive (a level is a column of a row).
    pub fn heads_of(module: i64, before: &[Rule], after: &[Rule], known: &Known, atoms: &[u8]) -> Result<(Vec<Head>, bool), Refused> {
        let side = if after.is_empty() { before } else { after };
        let mut heads: Vec<Head> = Vec::new();
        for r in side.iter().chain(before) {
            if heads.iter().any(|h| h.rel == r.head) {
                continue;
            }
            // A head's rules: the side's that write it, else (a head the after side no longer writes) the before side's.
            let rules = if side.iter().any(|q| q.head == r.head) { side } else { before };
            let sf: BTreeSet<bool> = rules.iter().filter(|q| q.head == r.head).map(|q| q.single_file(known)).collect();
            if sf.len() > 1 {
                return Err(Refused { module, rule: r.id, why: format!("relation {} is written by single-file and cross-file rules both", crate::sym::name_of(atoms, r.head)) });
            }
            let name = crate::sym::name_of(atoms, r.head);
            heads.push(Head { rel: r.head, arity: r.args.len(), counts: None, shape: head_shape(&name, r.args.len(), sf.contains(&true)) });
        }
        let own: BTreeSet<i64> = heads.iter().map(|h| h.rel).collect();
        let recursive = side.iter().any(|r| r.reads().any(|(_, rel)| own.contains(&rel)));
        // A recursive stratum's heads carry their levels (0364); a non-recursive one's, their derivation counts.
        for h in &mut heads {
            if recursive {
                if h.arity == 0 {
                    return Err(Refused { module, rule: side[0].id, why: format!("relation {} is nullary and recursive: a level is a column of a row (0364)", crate::sym::name_of(atoms, h.rel)) });
                }
                h.shape = levelled(&h.shape);
            } else {
                h.counts = Some(counts_shape(&h.shape));
            }
        }
        Ok((heads, recursive))
    }

    /// The input port relation `rel` is read on.
    fn port_of(&self, rel: i64) -> Option<usize> {
        self.ins.iter().position(|i| i.0 == rel).map(|p| p + usize::from(self.tape))
    }

    /// Head `k`'s output port: its trace.
    fn h(&self, k: usize) -> PortOut<Programmed> {
        PortOut::new(if self.recursive { k } else { 2 * k })
    }

    /// Head `k`'s derivation counts' output port (a non-recursive stratum's).
    fn c(&self, k: usize) -> PortOut<Programmed> {
        debug_assert!(!self.recursive);
        PortOut::new(2 * k + 1)
    }

    /// Runs `plan` into a runs block a head, read back: each head's derivations.
    /// Plans planner `p`'s chains and runs them into the heads' runs `runs` (`plan::Round`): the probes made.
    fn chains(&self, cx: &mut Cx<'_>, s: &Sides, p: &Planner<'_>, runs: &[BlockId], work: &mut Work) -> Result<u64, String> {
        let fresh = s.memo.is_none().then(|| crate::cache::OpCache::new(cx.dir).0);
        let memo = s.memo.or(fresh).expect("a memo");
        let round = Round { reg: cx.reg, body: &self.ops.body, ops: &self.ops.plan, atoms: s.atoms, memo, join: cx.join };
        let ran = round.run(cx.dir, &p.plan, &p.keys, runs);
        if let Some(m) = fresh {
            cx.dir.release(m);
        }
        let (probes, planned) = ran?;
        work.planned += planned;
        Ok(probes)
    }

    fn derive(&self, cx: &mut Cx<'_>, s: &Sides, p: &Planner<'_>, work: &mut Work) -> Result<Derived, String> {
        let mut cap = 256usize;
        loop {
            let runs: Vec<BlockId> = self.heads.iter().map(|h| Ok(open_runs(cx.dir, &h.shape.encode()?, cap))).collect::<Result<_, String>>()?;
            let ran = self.chains(cx, s, p, &runs, work);
            let read = ran.and_then(|probes| {
                work.probes += probes;
                let mut out = Vec::new();
                let mut need = 0;
                for &r in &runs {
                    let l = cx.dir.read("a stratum's derivations", &[r])?;
                    let rr = Runs::new(l.block(0));
                    need = need.max(rr.needed() + rr.len());
                    // copy: derived (a head's derivations, read back by the fixpoint's host loop)
                    out.push((0..rr.len()).map(|i| ((0..rr.head.shape.arity()).map(|c| rr.get(i, c)).collect(), rr.diff(i))).collect());
                }
                Ok((out, need))
            });
            runs.iter().for_each(|&r| cx.dir.release(r));
            let (out, need) = read?;
            if need > cap {
                cap = need + need / 4;
                continue;
            }
            work.rows_written += out.iter().map(Vec::len).sum::<usize>() as u64;
            return Ok(out);
        }
    }

    /// Δwave: the rules on each side, over `s`, into `out`'s heads and counts.
    fn wave(&self, cx: &mut Cx<'_>, s: &Sides, before: &[Rule], out: &mut Out<Programmed>) -> Result<Work, String> {
        let mut work = Work::default();
        let program_changed = before != self.after.as_slice();
        let sf = |r: &Rule| r.single_file(&self.known);
        let mut p = Planner::new(self, s);
        // Single-file rules at the file's grain: the documents changed, or whose relations changed.
        let mut files: BTreeSet<i64> = BTreeSet::new();
        if before.iter().chain(&self.after).any(sf) {
            if program_changed {
                files.extend(s.docs.iter().map(|d| d.d));
            } else {
                files.extend(s.docs.iter().filter(|d| d.before != d.after).map(|d| d.d));
                for r in self.after.iter().filter(|r| sf(r)) {
                    for (_, rel) in r.reads() {
                        if let Some(q) = self.port_of(rel) {
                            files.extend(&s.parts[q]);
                        }
                    }
                }
            }
        }
        let parts: Vec<i64> = files.iter().copied().collect();
        let head_of = |rel: i64| self.heads.iter().position(|h| h.rel == rel).expect("a head of the stratum");
        if !parts.is_empty() {
            for r in self.after.iter().filter(|r| sf(r)) {
                let order = cold(r, Side::After);
                p.chain(Walk { rule: r, order: &order, head: head_of(r.head), sign: 1, cand: None, below: None }, Docs::Of(&files), Docs::Of(&files), &parts);
            }
        }
        // Cross-file rules: removed ones cold on the before side, added ones on the after side, the rest by terms.
        for r in before.iter().filter(|r| !sf(r) && !self.after.contains(r)) {
            let order = cold(r, Side::Before);
            p.chain(Walk { rule: r, order: &order, head: head_of(r.head), sign: -1, cand: None, below: None }, Docs::All, Docs::All, &[]);
        }
        for r in self.after.iter().filter(|r| !sf(r)) {
            if !before.contains(r) {
                let order = cold(r, Side::After);
                p.chain(Walk { rule: r, order: &order, head: head_of(r.head), sign: 1, cand: None, below: None }, Docs::All, Docs::All, &[]);
                continue;
            }
            for (i, st) in r.steps.iter().enumerate() {
                if matches!(st.kind, Kind::Cmp(_)) || !self.changed(s, st.rel) {
                    continue;
                }
                let order = term_at(r, i);
                p.chain(Walk { rule: r, order: &order, head: head_of(r.head), sign: 1, cand: None, below: None }, Docs::All, Docs::Changed, &[]);
            }
        }
        // Each head's derivations: a file's retracted from its counts, then the chains'.
        let mut cap = 256usize;
        loop {
            let derivs: Vec<BlockId> = self.heads.iter().map(|h| Ok(open_runs(cx.dir, &h.shape.encode()?, cap))).collect::<Result<_, String>>()?;
            let ran = (|| {
                for (k, h) in self.heads.iter().enumerate() {
                    let sfh = before.iter().chain(&self.after).any(|r| r.head == h.rel && sf(r));
                    if let (true, false, Some(kept)) = (sfh, parts.is_empty(), out.kept(self.c(k))) {
                        work.rows_read += drop_counted(cx.reg, &self.ops.drops, cx.dir, kept, &parts, -1, derivs[k])?;
                    }
                }
                work.probes += self.chains(cx, s, &p, &derivs, &mut work)?;
                let mut need = 0;
                for &r in &derivs {
                    let l = cx.dir.read("a stratum's derivations", &[r])?;
                    let rr = Runs::new(l.block(0));
                    need = need.max(rr.len() + rr.needed());
                }
                if need > cap {
                    return Ok(Some(need));
                }
                for (k, h) in self.heads.iter().enumerate() {
                    let counts = out.kept(self.c(k));
                    work.rows_written += tally::run(cx.reg, &self.ops.tally, cx.dir, counts, h.arity, derivs[k], out.block(self.c(k)), out.block(self.h(k)))?;
                }
                Ok::<_, String>(None)
            })();
            derivs.iter().for_each(|&r| cx.dir.release(r));
            match ran? {
                Some(need) => cap = need + need / 4,
                None => return Ok(work),
            }
        }
    }

    /// Whether the source of relation `rel` changed in this run.
    fn changed(&self, s: &Sides, rel: i64) -> bool {
        if self.known.view(rel).is_some() {
            return s.docs.iter().any(|d| d.before != d.after);
        }
        self.port_of(rel).is_some_and(|q| s.changed[q])
    }

    /// The rows of each head's working state, by their key: each one's level.
    fn levels(&self, cx: &Cx<'_>, w: &[Trace]) -> Result<Found, String> {
        let mut out = Vec::with_capacity(w.len());
        for t in w {
            let mut m = BTreeMap::new();
            for mut r in rows_at(cx.dir, t, t.head())? {
                let l = r.pop().expect("a level");
                m.insert(r, l);
            }
            out.push(m);
        }
        Ok(out)
    }

    /// Pushes `rows` (`(key, level, ±1)`) into a head's working state, over `base`, as a layer.
    fn put(&self, cx: &mut Cx<'_>, w: &mut Trace, base: Option<BlockId>, rows: &[(Vec<i64>, i64, i32)]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let pairs: Vec<(Vec<i64>, i32)> = rows.iter().map(|(v, l, d)| (v.iter().copied().chain([*l]).collect(), *d)).collect(); // copy: derived (a round's rows and their levels, written once)
        crate::contract::push_layer_over(cx.reg, cx.traces, cx.dir, w, &pairs, base)?;
        Ok(())
    }

    /// The plan's delta terms over the own heads' change: for each own atom of each rule, that step on the change of
    /// the working states, the other own atoms on `own` and the other relations on `other`.
    fn own_terms(&self, p: &mut Planner<'_>, own: Side, other: Side) {
        let is_own = |rel: i64| self.heads.iter().position(|h| h.rel == rel);
        for r in &self.after {
            for (i, st) in r.steps.iter().enumerate() {
                if st.kind != Kind::Atom || is_own(st.rel).is_none() {
                    continue;
                }
                let order: Vec<(usize, Side)> = std::iter::once((i, Side::Delta))
                    .chain((0..r.steps.len()).filter(|&j| j != i).map(|j| (j, if is_own(r.steps[j].rel).is_some() { own } else { other })))
                    .collect();
                p.chain(Walk { rule: r, order: &order, head: is_own(r.head).expect("own"), sign: 1, cand: None, below: None }, Docs::All, Docs::All, &[]);
            }
        }
    }

    /// Δfix by levels (0364 §2), over `s`, into `out`: each head's working state over where its trace was kept (from ∅
    /// when the program changed), and its change since then the output's.
    fn fix(&self, cx: &mut Cx<'_>, s: &Sides, before: &[Rule], out: &mut Out<Programmed>) -> Result<Work, String> {
        let mut work = Work::default();
        let program_changed = before != self.after.as_slice();
        let base: Vec<Option<BlockId>> = (0..self.heads.len()).map(|k| if program_changed { None } else { out.kept(self.h(k)) }).collect();
        let mut w: Vec<Trace> = self.heads.iter().zip(&base).map(|(h, &b)| Trace::over(&h.shape, b)).collect::<Result<_, _>>()?;
        let ran = self.fix_in(cx, s, program_changed, &mut w, &base, &mut work);
        let written = ran.and_then(|()| self.fix_out(cx, out, &w, program_changed));
        w.iter_mut().zip(&base).for_each(|(t, &b)| t.discard_over(cx.dir, b));
        written.map(|()| work)
    }

    #[allow(clippy::too_many_arguments)]
    fn fix_in(&self, cx: &mut Cx<'_>, s: &Sides, program_changed: bool, w: &mut [Trace], base: &[Option<BlockId>], work: &mut Work) -> Result<(), String> {
        let n = self.heads.len();
        let own = |rel: i64| self.heads.iter().position(|h| h.rel == rel);
        let heads_of = |w: &[Trace]| w.iter().map(Trace::head).collect::<Vec<_>>();
        let mut now = self.levels(cx, w)?;
        // A plan over the working states between `from` and `to`.
        let planner = |from: &[Option<BlockId>], to: &[Option<BlockId>]| {
            let mut p = Planner::new(self, s);
            p.own = from.iter().copied().zip(to.iter().copied()).collect();
            p
        };
        let mut seeds: Found = vec![BTreeMap::new(); n];
        if program_changed {
            // From ∅: every rule over the inputs' after side; a row no own row derives is of level 0.
            let hs = heads_of(w);
            let mut p = planner(&hs, &hs);
            for r in &self.after {
                let order = cold(r, Side::After);
                p.chain(Walk { rule: r, order: &order, head: own(r.head).expect("own"), sign: 1, cand: None, below: None }, Docs::All, Docs::All, &[]);
            }
            found(&mut seeds, self.derive(cx, s, &p, work)?);
        } else {
            // Candidates (0364 §2.1): the rows that lost a derivation through a gone input row, the derivation on the
            // inputs' before side and the heads as kept.
            let hs = heads_of(w);
            let mut p = planner(&hs, &hs);
            let mut ins_terms = Vec::new();
            for r in &self.after {
                for (i, st) in r.steps.iter().enumerate() {
                    if matches!(st.kind, Kind::Cmp(_)) || own(st.rel).is_some() || !self.changed(s, st.rel) {
                        continue;
                    }
                    ins_terms.push((r, i));
                    let order: Vec<(usize, Side)> = std::iter::once((i, Side::Delta)).chain((0..r.steps.len()).filter(|&j| j != i).map(|j| (j, if own(r.steps[j].rel).is_some() { Side::After } else { Side::Before }))).collect();
                    p.chain(Walk { rule: r, order: &order, head: own(r.head).expect("own"), sign: 1, cand: None, below: None }, Docs::All, Docs::Changed, &[]);
                }
            }
            let mut cands: BTreeMap<i64, Vec<BTreeSet<Vec<i64>>>> = BTreeMap::new();
            let candidate = |cands: &mut BTreeMap<i64, Vec<BTreeSet<Vec<i64>>>>, now: &Found, k: usize, mut row: Vec<i64>, above: i64| {
                row.pop();
                if let Some(&l) = now[k].get(&row)
                    && l > above
                {
                    cands.entry(l).or_insert_with(|| vec![BTreeSet::new(); n])[k].insert(row);
                }
            };
            for (k, rows) in self.derive(cx, s, &p, work)?.into_iter().enumerate() {
                for (row, d) in rows {
                    if d < 0 {
                        candidate(&mut cands, &now, k, row, i64::MIN);
                    }
                }
            }
            // In level order, lowest first (§2.2): a candidate stays when a derivation that still holds reads only
            // own rows present and of lower level; else it goes for now, and the rows that read it are candidates
            // at their own levels (§2.3), which are above its.
            let mut gone: Vec<BTreeSet<Vec<i64>>> = vec![BTreeSet::new(); n];
            while let Some((l, batch)) = cands.pop_first() {
                let stays = self.recheck(cx, s, w, &batch, Some(l), work)?;
                let from = heads_of(w);
                let mut any = false;
                for (k, rows) in batch.iter().enumerate() {
                    let dead: Vec<(Vec<i64>, i64, i32)> = rows.iter().filter(|r| !stays[k].contains_key(*r)).map(|r| (r.clone(), l, -1)).collect(); // copy: derived (the rows that go for now)
                    any |= !dead.is_empty();
                    for (r, _, _) in &dead {
                        now[k].remove(r);
                        gone[k].insert(r.clone());
                    }
                    self.put(cx, &mut w[k], base[k], &dead)?;
                }
                if !any {
                    continue;
                }
                let mut p = planner(&from, &heads_of(w));
                self.own_terms(&mut p, Side::Before, Side::After);
                for (k, rows) in self.derive(cx, s, &p, work)?.into_iter().enumerate() {
                    for (row, d) in rows {
                        if d < 0 {
                            candidate(&mut cands, &now, k, row, l);
                        }
                    }
                }
            }
            // The rows gone for now, levelled from the rows present (§2.4): one still derived comes back, at the
            // level its derivation gives (a rewrite).
            if gone.iter().any(|g| !g.is_empty()) {
                let back = self.recheck(cx, s, w, &gone, None, work)?;
                for (k, m) in back.into_iter().enumerate() {
                    seeds[k].extend(m);
                }
            }
            // The rows the edit adds: derived through an added input row, from the rows present.
            let hs = heads_of(w);
            let mut p = planner(&hs, &hs);
            for (r, i) in ins_terms {
                let order: Vec<(usize, Side)> = std::iter::once((i, Side::Delta)).chain((0..r.steps.len()).filter(|&j| j != i).map(|j| (j, Side::After))).collect();
                p.chain(Walk { rule: r, order: &order, head: own(r.head).expect("own"), sign: 1, cand: None, below: None }, Docs::All, Docs::Changed, &[]);
            }
            found(&mut seeds, self.derive(cx, s, &p, work)?);
        }
        // Rounds: the rows found that are not present added at the level their derivation gives (one more than the
        // highest it reads: added rows never lower a level, §3), then what they derive, until a round adds nothing.
        loop {
            let from = heads_of(w);
            let mut any = false;
            for (k, rows) in seeds.iter().enumerate() {
                let new: Vec<(Vec<i64>, i64, i32)> = rows.iter().filter(|(r, _)| !now[k].contains_key(*r)).map(|(r, &l)| (r.clone(), l, 1)).collect(); // copy: derived (a round's new rows)
                any |= !new.is_empty();
                for (r, l, _) in &new {
                    now[k].insert(r.clone(), *l);
                }
                self.put(cx, &mut w[k], base[k], &new)?;
            }
            if !any {
                return Ok(());
            }
            let mut p = planner(&from, &heads_of(w));
            self.own_terms(&mut p, Side::After, Side::After);
            seeds = vec![BTreeMap::new(); n];
            found(&mut seeds, self.derive(cx, s, &p, work)?);
        }
    }

    /// Of `cands` (a head's keys each), those with a derivation from the working states as they are, through own rows
    /// of level below `below` where it is given: each with the level that derivation gives.
    fn recheck(&self, cx: &mut Cx<'_>, s: &Sides, w: &[Trace], cands: &[BTreeSet<Vec<i64>>], below: Option<i64>, work: &mut Work) -> Result<Found, String> {
        let hs: Vec<Option<BlockId>> = w.iter().map(Trace::head).collect();
        let mut temps: Vec<Trace> = Vec::new();
        let ran = (|| {
            let mut p = Planner::new(self, s);
            p.own = hs.iter().map(|&h| (h, h)).collect();
            for (k, rows) in cands.iter().enumerate() {
                if rows.is_empty() {
                    continue;
                }
                let mut t = Trace::new(&key_shape(&self.heads[k].shape))?.with_roll_up(RollUp::Never);
                let pairs: Vec<(Vec<i64>, i32)> = rows.iter().map(|r| (r.clone(), 1)).collect(); // copy: derived (the candidates, a relation the recheck starts from)
                let head = crate::contract::push_layer(cx.reg, cx.traces, cx.dir, &mut t, &pairs)?;
                temps.push(t);
                let src = p.src(Src::Trace { arity: self.heads[k].arity, before: None, after: Some(head) }, 0);
                for r in self.after.iter().filter(|r| r.head == self.heads[k].rel) {
                    let order = cold(r, Side::After);
                    p.chain(Walk { rule: r, order: &order, head: k, sign: 1, cand: Some(src), below }, Docs::All, Docs::All, &[]);
                }
            }
            let mut out: Found = vec![BTreeMap::new(); self.heads.len()];
            found(&mut out, self.derive(cx, s, &p, work)?);
            Ok::<_, String>(out)
        })();
        temps.iter_mut().for_each(|t| t.clear(cx.dir));
        ran
    }

    /// The working states' net change since the kept heads, into `out`: rows and their levels, a rewrite a kill and an
    /// add (0364 §4). A working state over the kept head is read as their change; one made from ∅ (a changed program)
    /// is diffed with it.
    fn fix_out(&self, cx: &mut Cx<'_>, out: &mut Out<Programmed>, w: &[Trace], fresh: bool) -> Result<(), String> {
        for (k, t) in w.iter().enumerate() {
            let kept = out.kept(self.h(k));
            let ar = self.heads[k].arity;
            let (gone, added): (Vec<Vec<i64>>, Vec<Vec<i64>>) = if fresh {
                let (b, a) = (rows_at(cx.dir, t, kept)?, rows_at(cx.dir, t, t.head())?);
                (b.difference(&a).cloned().collect(), a.difference(&b).cloned().collect())
            } else {
                let l = cx.dir.read("a fixpoint's change", &[kept, t.head()].into_iter().flatten().collect::<Vec<_>>())?;
                let table = l.table();
                let (b, a) = (t.state(&table, kept)?, t.state(&table, t.head())?);
                let change = Bistate::new(b, a);
                let row = |v: &View<'_>, id: RowId| (0..=ar).map(|c| cell(v, id, c)).collect::<Vec<i64>>(); // copy: derived (a changed row, into the output's runs)
                (change.gone().map(|id| row(&b, id)).collect(), change.added().map(|id| row(&a, id)).collect())
            };
            let h = out.writer(cx.dir, self.h(k))?;
            for r in &gone {
                h.push(r, -1);
            }
            for r in &added {
                h.push(r, 1);
            }
        }
        Ok(())
    }

    fn sides_in(&self, cx: &Cx<'_>, ins: &In<'_, Programmed>) -> Result<Sides, String> {
        let n = self.ins.len() + usize::from(self.tape);
        let heads = (0..n).map(|p| ins.heads(PortIn::new(p))).collect();
        let changed = (0..n).map(|p| !ins.port(PortIn::new(p)).is_identity()).collect();
        let parts = (0..n)
            .map(|p| {
                let b = ins.port(PortIn::new(p));
                let mut set: BTreeSet<i64> = b.added().map(|id| cell(b.after(), id, 0)).collect();
                if let Some(bb) = b.before() {
                    set.extend(b.gone().map(|id| cell(bb, id, 0)));
                }
                set
            })
            .collect();
        let docs = if self.tape {
            let t = ins.port(PortIn::new(0));
            docs_of(cx, t.before(), t.after())?
        } else {
            Vec::new()
        };
        Ok(Sides { heads, changed, parts, docs, atoms: ins.global(crate::contract::PortGlobal::new(0)).block, memo: None })
    }
}

impl DeltaOp for Stratum {
    type Ports = Programmed;
    type Cache = crate::plan::Memo;

    fn ports(&self) -> Result<Ports, String> {
        let ins = self.tape.then(|| Port { name: "documents", shape: tape_shape() }).into_iter().chain(self.ins.iter().map(|(_, s)| Port { name: "relation", shape: s.clone() })).collect();
        let outs = self.heads.iter().flat_map(|h| std::iter::once(Port { name: "head", shape: h.shape.clone() }).chain(h.counts.clone().map(|shape| Port { name: "derivation counts", shape }))).collect();
        Ok(Ports { ins, globals: vec!["atoms"], outs })
    }

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, Programmed>, _want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<crate::plan::Memo>, out: &mut Out<Programmed>, _asks: &mut Asks<Programmed>) -> Result<Work, String> {
        if ins.is_identity() && self.before == self.after {
            return Ok(Work::default());
        }
        let mut s = self.sides_in(cx, ins)?;
        s.memo = Some(cache.write(cx)?);
        // Nothing kept and every input from ∅: the stratum has not run (a fresh instance's outputs are ∅, which is
        // op(∅) only for a program with no facts and no leading filters), so it runs cold, as from no rules (L2).
        let n = self.ins.len() + usize::from(self.tape);
        let fresh = (0..self.heads.len()).all(|k| out.kept(self.h(k)).is_none()) && (0..n).all(|p| ins.heads(PortIn::new(p)).0.is_none());
        let before: &[Rule] = if fresh { &[] } else { &self.before };
        if self.recursive { self.fix(cx, &s, before, out) } else { self.wave(cx, &s, before, out) }
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, Programmed>, out: &mut Out<Programmed>) -> Result<(), String> {
        let n = self.ins.len() + usize::from(self.tape);
        let heads = (0..n).map(|p| (None, ins.head(PortIn::new(p)))).collect();
        let docs = if self.tape { docs_of(cx, None, ins.port(PortIn::new(0)))? } else { Vec::new() };
        let s = Sides { heads, changed: vec![true; n], parts: vec![BTreeSet::new(); n], docs, atoms: ins.global(crate::contract::PortGlobal::new(0)).block, memo: None };
        if self.recursive { self.fix(cx, &s, &[], out) } else { self.wave(cx, &s, &[], out) }.map(drop)
    }
}

/// A trace between two of its heads, a run's before and after (`None`: ∅).
pub type Between<'a> = (&'a Trace, Option<BlockId>, Option<BlockId>);

/// A relation given to a level as a fact in: its trace and shape (the caller's, or an earlier level's).
pub struct FactIn<'a> {
    pub rel: i64,
    pub trace: &'a Trace,
    pub shape: Shape,
}

/// A level's relation, kept across runs: its trace, and its derivation counts' (a non-recursive stratum's head's).
pub struct Rel {
    pub shape: Shape,
    pub head: Trace,
    pub counts: Option<Trace>,
}

impl Rel {
    fn heads(&self) -> (Option<BlockId>, Option<BlockId>) {
        (self.head.head(), self.counts.as_ref().and_then(Trace::head))
    }

    fn clear(&mut self, dir: &mut Directory) {
        self.head.clear(dir);
        if let Some(c) = &mut self.counts {
            c.clear(dir);
        }
    }

    /// Whether it holds head `h`'s traces: the same shapes.
    fn fits(&self, h: &Head) -> bool {
        self.shape == h.shape && self.counts.is_some() == h.counts.is_some()
    }
}

/// Where a level's run left its traces: its documents' head, each relation's head and companion's, and each fact
/// in's head when it last read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelPins {
    pub tapes: Option<BlockId>,
    pub rels: Vec<(i64, Option<BlockId>, Option<BlockId>)>,
    pub seen: Vec<(i64, Option<BlockId>)>,
}

impl LevelPins {
    /// Its words, as a kept pins block holds them.
    pub fn words(&self) -> Vec<i64> {
        let w = |h: Option<BlockId>| h.map_or(-1, |b| i64::from(b.0));
        let mut out = vec![w(self.tapes), self.rels.len() as i64];
        out.extend(self.rels.iter().flat_map(|&(r, h, c)| [r, w(h), w(c)]));
        out.push(self.seen.len() as i64);
        out.extend(self.seen.iter().flat_map(|&(r, h)| [r, w(h)]));
        out
    }

    /// The pins its words hold, from `next`, a word a call.
    pub fn of_words(next: &mut dyn FnMut() -> Result<i64, String>) -> Result<LevelPins, String> {
        let h = |x: i64| (x >= 0).then_some(BlockId(x as u32));
        let tapes = h(next()?);
        let rels = (0..next()?).map(|_| Ok((next()?, h(next()?), h(next()?)))).collect::<Result<_, String>>()?;
        let seen = (0..next()?).map(|_| Ok((next()?, h(next()?)))).collect::<Result<_, String>>()?;
        Ok(LevelPins { tapes, rels, seen })
    }

    /// Every head it names.
    pub fn heads(&self) -> impl Iterator<Item = BlockId> + '_ {
        std::iter::once(self.tapes).chain(self.rels.iter().flat_map(|&(_, h, c)| [h, c])).chain(self.seen.iter().map(|s| s.1)).flatten()
    }
}

/// One level of B§8's loop: a phase over its program and facts in.
pub struct Level {
    /// Where its traces' layers are placed (`Directory::lasting`).
    place: u32,
    pub rels: BTreeMap<i64, Rel>,
    /// The program it last ran.
    pub ran: Program,
    /// Its documents (its `load` answers): rows of the document set's shape.
    pub tapes: Trace,
    /// Each fact in's head when it last read it.
    pub seen: BTreeMap<i64, Option<BlockId>>,
    /// This run's start: each relation's head and the documents', where its roll-up starts.
    start: BTreeMap<i64, (Option<BlockId>, Option<BlockId>)>,
    tapes_start: Option<BlockId>,
    /// `load`'s symbol: the level's asks, kept out of its facts out.
    load: Option<i64>,
    /// Each module's operator cache, its planner's memo (Operations §6.1: "in memory across runs"): kept from one
    /// run of the level to the next, let go with its module.
    caches: BTreeMap<i64, crate::cache::OpCache>,
}

/// A level's run: its work, and whether it exported a program.
pub struct LevelRun {
    pub work: Work,
    pub program: bool,
    /// Whether it ran from ∅ (a head's shape changed): the levels after it then run from ∅ too.
    pub reset: bool,
}

impl Level {
    /// An empty level, its layers placed at `place`.
    pub fn new(place: u32) -> Result<Level, String> {
        let tapes = Trace::new(&tape_shape())?.with_roll_up(RollUp::Never).resumed(place, None);
        Ok(Level { place, rels: BTreeMap::new(), ran: Program::default(), tapes, seen: BTreeMap::new(), start: BTreeMap::new(), tapes_start: None, load: None, caches: BTreeMap::new() })
    }

    /// Its relation `rel`'s trace between this run's start and now.
    pub fn between(&self, rel: i64) -> Option<Between<'_>> {
        let r = self.rels.get(&rel)?;
        let start = self.start.get(&rel).map_or(r.head.head(), |s| s.0);
        Some((&r.head, start, r.head.head()))
    }

    /// Its K-form relations (`ksyms`, their symbols), a program for the next level: each one's trace between this
    /// run's start and now (`None`: the level writes none).
    pub fn program(&self, ksyms: &[Option<i64>]) -> Vec<Option<Between<'_>>> {
        ksyms.iter().map(|s| s.and_then(|s| self.between(s))).collect()
    }

    /// Whether it exports a program (O6): a K-form relation it writes holds rows.
    pub fn exports(&self, dir: &Directory, ksyms: &[Option<i64>]) -> Result<bool, String> {
        for (t, _, h) in self.program(ksyms).into_iter().flatten() {
            if !rows_at(dir, t, h)?.is_empty() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Its facts out, the next level's facts in (O6): every relation it writes that is not K-form, and not its own
    /// `load` facts, which this level answered.
    pub fn facts(&self, ksyms: &[Option<i64>]) -> Vec<FactIn<'_>> {
        self.rels.iter().filter(|(r, _)| !ksyms.contains(&Some(**r)) && self.load != Some(**r)).map(|(&rel, r)| FactIn { rel, trace: &r.head, shape: r.shape.clone() }).collect()
    }

    /// Each fact in seen where it is now (a run's end, after the level before it rolled its traces up).
    pub fn saw(&mut self, facts: &[FactIn<'_>]) {
        for f in facts {
            self.seen.insert(f.rel, f.trace.head());
        }
    }

    /// The documents it reads (its `load` answers').
    pub fn docs(&self, dir: &Directory) -> Result<BTreeSet<i64>, String> {
        Ok(rows_at(dir, &self.tapes, self.tapes.head())?.into_iter().map(|r| r[0]).collect())
    }

    /// Its traces let go (a cold run's start), and its modules' caches.
    pub fn clear(&mut self, dir: &mut Directory) {
        for r in self.rels.values_mut() {
            r.clear(dir);
        }
        self.tapes.clear(dir);
        self.let_caches_go(dir);
    }

    fn let_caches_go(&mut self, dir: &mut Directory) {
        for (_, c) in std::mem::take(&mut self.caches) {
            dir.release(c.0);
        }
    }

    /// Its relations let go and its program forgotten, its documents kept: the next run of it is from ∅ but for them.
    fn reset(&mut self, dir: &mut Directory) {
        for r in self.rels.values_mut() {
            r.clear(dir);
        }
        self.rels.clear();
        self.let_caches_go(dir);
        self.ran = Program::default();
        self.seen.clear();
        self.start.clear();
    }

    /// Δphase: the program `program` (K-form's eight relations, each between two heads; `None`: ∅) over facts in
    /// `facts`, its documents the `load` answers those facts ask (through `docs`), its strata in `k_order`.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, ops: PhaseOps, bench: &mut Bench, docs: &mut DocSet, leaves: &mut LeavesW, atoms: Atoms, program: &[Option<Between<'_>>], facts: &[FactIn<'_>]) -> Result<LevelRun, String> {
        let mut work = Work::default();
        // This run's start, where each of its traces is.
        self.start = self.rels.iter().map(|(&r, t)| (r, t.heads())).collect();
        self.tapes_start = self.tapes.head();
        // The program, as rules.
        let rows = program_rows(&bench.dir, program)?;
        let (next, known) = {
            let l = bench.dir.read("atoms", &[atoms.0])?;
            (Program::read(&rows, l.block(0)).map_err(|e| e.named(&docs.path_of(&bench.dir, atoms, e.module)))?, Known::new(l.block(0)))
        };
        // Its documents: the `load` answers its facts in ask, and its program's `load` facts (Bootstrap §5: "its load
        // facts ask the loader"; P3 loads what L3 reads).
        self.load = known.load;
        self.answer(bench, docs, leaves, atoms, &known, facts, &next).map_err(|e| format!("a level's documents: {e}"))?;
        // A head whose shape the edit changed (its grain, its stratum's recursion): a trace keeps one shape, so the
        // level runs from ∅, its documents kept.
        let mut reset = false;
        {
            let l = bench.dir.read("atoms", &[atoms.0])?;
            'modules: for (&m, md) in &next.modules {
                let before = self.ran.modules.get(&m).map_or(&[][..], |b| &b.rules[..]);
                let (heads, _) = Stratum::heads_of(m, before, &md.rules, &known, l.block(0)).map_err(|e| e.named(&docs.path_of(&bench.dir, atoms, m)))?;
                for h in &heads {
                    if self.rels.get(&h.rel).is_some_and(|r| !r.fits(h)) {
                        reset = true;
                        break 'modules;
                    }
                }
            }
        }
        if reset {
            self.reset(&mut bench.dir);
        }
        // Its strata: modules gone first (their rows retracted), then the program's in k_order.
        let gone: Vec<i64> = self.ran.modules.keys().filter(|m| !next.modules.contains_key(m)).copied().collect();
        let order: Vec<i64> = gone.iter().copied().chain(next.strata()).collect();
        // The strata that ran, in order: each one's module, the relations it writes and those it reads.
        let mut strata: Vec<(i64, Vec<i64>, Vec<i64>)> = Vec::new();
        // The facts the strata's rules give their heads (0361), as they run: each from the declarations and the facts
        // inferred before it.
        let mut inferred: Shapes = BTreeMap::new();
        for m in order {
            let before = self.ran.modules.get(&m).map_or(Vec::new(), |md| md.rules.clone());
            let after = next.modules.get(&m).map_or(Vec::new(), |md| md.rules.clone());
            let (w, writes, reads) = self.run_module(ops, bench, atoms, &known, m, before, after, facts, &inferred).map_err(|e| format!("module {}: {e}", docs.path_of(&bench.dir, atoms, m)))?;
            work += w;
            if let Some(md) = next.modules.get(&m) {
                strata.push((m, writes.clone(), reads));
                let declared = self.declared(&bench.dir, &known, facts)?;
                let of = |r: i64| match (declared.get(&r), inferred.get(&r)) {
                    (Some(a), Some(b)) => Some(a.and(b)),
                    (a, b) => a.or(b).cloned(),
                };
                let recursive = md.rules.iter().any(|r| r.reads().any(|(_, rel)| writes.contains(&rel)));
                let got: Vec<(i64, Facts)> = writes
                    .iter()
                    .map(|&h| {
                        let rules: Vec<&Rule> = md.rules.iter().filter(|r| r.head == h).collect();
                        (h, crate::shapes::infer(h, &rules, recursive, &of))
                    })
                    .filter(|(_, f)| !f.is_empty())
                    .collect();
                inferred.extend(got);
            }
        }
        // The run's critical path over its strata (0203 §4): each one's producers the strata before it that write a
        // relation it reads.
        let producers: Vec<Vec<usize>> = strata.iter().enumerate().map(|(j, (_, _, reads))| (0..j).filter(|&p| strata[p].1.iter().any(|r| reads.contains(r))).collect()).collect();
        let memos: Vec<BlockId> = strata.iter().filter_map(|(m, _, _)| self.caches.get(m).map(|c| c.0)).collect();
        if memos.len() == strata.len() {
            work.planned.critical += ops.plan.critical_path(&bench.reg, &mut bench.dir, &memos, &producers)?;
        }
        self.ran = next;
        for f in facts {
            self.seen.insert(f.rel, f.trace.head());
        }
        let ksyms = kform_syms(&bench.dir, atoms);
        let program = self.exports(&bench.dir, &ksyms)?;
        Ok(LevelRun { work, program, reset })
    }

    /// The facts the level's relations declare now (0361): its `key` and `fanout` rows, a relation of the level's or a
    /// fact in.
    fn declared(&self, dir: &Directory, known: &Known, facts: &[FactIn<'_>]) -> Result<Shapes, String> {
        let rows = |rel: Option<i64>| -> Result<BTreeSet<Vec<i64>>, String> {
            let Some(rel) = rel else { return Ok(BTreeSet::new()) };
            match self.rels.get(&rel).map(|r| &r.head).or_else(|| facts.iter().find(|f| f.rel == rel).map(|f| f.trace)) {
                Some(t) => rows_at(dir, t, t.head()),
                None => Ok(BTreeSet::new()),
            }
        };
        Ok(crate::shapes::declared(&rows(known.key)?, &rows(known.fanout)?, known.one, known.few))
    }

    /// The level's documents moved to the `load` rows of its facts in and of its program's facts: each path read
    /// through the document set (its load, parse and intern answers), and its row in the level's documents. The
    /// documents are read before the rules run, so a `load` the program derives (a rule with a body) is refused.
    #[allow(clippy::too_many_arguments)]
    fn answer(&mut self, bench: &mut Bench, docs: &mut DocSet, leaves: &mut LeavesW, atoms: Atoms, known: &Known, facts: &[FactIn<'_>], program: &Program) -> Result<(), String> {
        let mut asked: BTreeSet<Vec<i64>> = BTreeSet::new();
        if let Some(load) = known.load {
            if let Some(f) = facts.iter().find(|f| f.rel == load) {
                asked.extend(rows_at(&bench.dir, f.trace, f.trace.head())?);
            }
            for r in program.modules.values().flat_map(|m| &m.rules).filter(|r| r.head == load) {
                let row: Option<Vec<i64>> = r.args.iter().map(|a| if let Term::Const(v) = a { Some(*v) } else { None }).collect();
                match row {
                    Some(row) if r.steps.is_empty() && row.len() == 3 => {
                        asked.insert(row);
                    }
                    _ => return Err(format!("module {} rule {}: a load that is not a fact of a path, a module and a grammar: a level reads its documents before its rules run", docs.path_of(&bench.dir, atoms, r.module), r.id)),
                }
            }
        }
        let mut want: BTreeSet<Vec<i64>> = BTreeSet::new();
        for r in asked {
            let (path, module) = (atoms.name(&bench.dir, r[0]), atoms.name(&bench.dir, r[1]));
            docs.read(bench, leaves, atoms, &path, &module)?;
            if let Some(row) = docs.row_of(&bench.dir, atoms, &path)? {
                want.insert(row);
            }
        }
        docs.resident(bench)?;
        let have = rows_at(&bench.dir, &self.tapes, self.tapes.head())?;
        let pairs: Vec<(Vec<i64>, i32)> = have.difference(&want).map(|r| (r.clone(), -1)).chain(want.difference(&have).map(|r| (r.clone(), 1))).collect(); // copy: derived (the level's documents' change)
        if !pairs.is_empty() {
            crate::contract::push_layer(&bench.reg, &bench.traces, &mut bench.dir, &mut self.tapes, &pairs)?;
        }
        Ok(())
    }

    /// One stratum's run: module `m`'s rules `before` and `after`, its inputs each between where this level last read
    /// it and where it is now.
    #[allow(clippy::too_many_arguments)]
    fn run_module(&mut self, ops: PhaseOps, bench: &mut Bench, atoms: Atoms, known: &Known, m: i64, before: Vec<Rule>, after: Vec<Rule>, facts: &[FactIn<'_>], inferred: &Shapes) -> Result<(Work, Vec<i64>, Vec<i64>), String> {
        let mut shapes: BTreeMap<i64, Shape> = facts.iter().map(|f| (f.rel, f.shape.clone())).collect();
        shapes.extend(self.rels.iter().map(|(&r, t)| (r, t.shape.clone())));
        let after_empty = after.is_empty();
        let st = {
            let l = bench.dir.read("atoms", &[atoms.0])?;
            Stratum::new(ops, m, before, after, known.clone(), &shapes, l.block(0)).map_err(|e| format!("rule {}: {}", e.rule, e.why))?.with_inferred(inferred)
        };
        for h in &st.heads {
            if !self.rels.contains_key(&h.rel) {
                let head = Trace::new(&h.shape)?.with_roll_up(RollUp::Never).resumed(self.place, None);
                let counts = h.counts.as_ref().map(|c| Ok::<_, String>(Trace::new(c)?.with_roll_up(RollUp::Never).resumed(self.place, None))).transpose()?;
                self.rels.insert(h.rel, Rel { shape: h.shape.clone(), head, counts });
            }
        }
        // Its inputs, each between two heads.
        let mut ins: Vec<(&Trace, Option<BlockId>, Option<BlockId>)> = Vec::new();
        if st.tape {
            ins.push((&self.tapes, self.tapes_start, self.tapes.head()));
        }
        let mut taken: Vec<(i64, Rel)> = st.heads.iter().map(|h| (h.rel, self.rels.remove(&h.rel).expect("made"))).collect();
        let mut run = || {
            for (rel, _) in &st.ins {
                if let Some(r) = self.rels.get(rel) {
                    let start = self.start.get(rel).map_or(r.head.head(), |s| s.0);
                    ins.push((&r.head, start, r.head.head()));
                } else if let Some(f) = facts.iter().find(|f| f.rel == *rel) {
                    ins.push((f.trace, self.seen.get(rel).copied().flatten(), f.trace.head()));
                }
            }
            let count = u64::from(atoms.count(&bench.dir));
            let globals = [GlobalIn { block: atoms.0, before: Some(count), after: count }];
            let outs: Vec<Trace> = taken.iter_mut().flat_map(|(_, r)| std::iter::once(std::mem::replace(&mut r.head, placeholder())).chain(r.counts.as_mut().map(|c| std::mem::replace(c, placeholder())))).collect();
            let cache = match self.caches.remove(&m) {
                Some(c) => c,
                None => crate::cache::OpCache::new(&mut bench.dir),
            };
            let mut inst = Instance { outs, cache, want: placeholder(), asks: (0..ins.len()).map(|_| None).collect() };
            let w = inst.step(&st, &mut bench.cx(), &ins, &globals, None);
            // A module gone lets its cache go; any other keeps it for its next run.
            if after_empty {
                bench.dir.release(inst.cache.0);
            } else {
                self.caches.insert(m, inst.cache);
            }
            let mut outs = inst.outs.into_iter();
            for (_, r) in &mut taken {
                r.head = outs.next().expect("a head's trace");
                if let Some(c) = &mut r.counts {
                    *c = outs.next().expect("a head's counts' trace");
                }
            }
            w
        };
        let ran = run()?;
        for (rel, r) in taken {
            self.rels.insert(rel, r);
        }
        Ok((ran, st.heads.iter().map(|h| h.rel).collect(), st.ins.iter().map(|i| i.0).collect()))
    }

    /// The run's end: every trace's layers since the run's start rolled up.
    pub fn finish(&mut self, bench: &mut Bench) -> Result<(), String> {
        for (rel, r) in &mut self.rels {
            let (h, c) = self.start.get(rel).copied().unwrap_or((None, None));
            r.head.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, h, crate::op::Tiling::WHOLE)?;
            if let Some(t) = &mut r.counts {
                t.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, c, crate::op::Tiling::WHOLE)?;
            }
        }
        self.tapes.roll_up_run(&bench.reg, &bench.traces, &mut bench.dir, self.tapes_start, crate::op::Tiling::WHOLE)?;
        self.start.clear();
        self.tapes_start = self.tapes.head();
        Ok(())
    }

    /// Where its traces are, for a run's pins: its documents', each relation's that holds a layer, and each fact in's
    /// head when it last read it.
    pub fn pins(&self) -> LevelPins {
        let rels = self.rels.iter().map(|(&r, t)| (r, t.heads())).filter(|(_, (h, c))| h.is_some() || c.is_some()).map(|(r, (h, c))| (r, h, c)).collect();
        LevelPins { tapes: self.tapes.head(), rels, seen: self.seen.iter().map(|(&r, &h)| (r, h)).collect() }
    }

    /// The level a kept run left at `pins`, its program `ran` (the one the level before it left), its layers placed
    /// at `place`: each trace resumed at its pinned head, its shape its head layer's own.
    pub fn resumed(dir: &Directory, place: u32, pins: &LevelPins, ran: Program) -> Result<Level, String> {
        let mut l = Level::new(place)?;
        l.tapes = l.tapes.resumed(place, pins.tapes);
        for &(rel, h, c) in &pins.rels {
            let Some(h) = h else { return Err(format!("relation {rel}: a kept level's counts without its rows")) };
            let head = Trace::at_layer(dir, place, h)?;
            let counts = c.map(|c| Trace::at_layer(dir, place, c)).transpose()?;
            let shape = rows_shape(dir, h)?;
            l.rels.insert(rel, Rel { shape, head, counts });
        }
        l.seen = pins.seen.iter().copied().collect();
        l.ran = ran;
        l.tapes_start = l.tapes.head();
        Ok(l)
    }
}

/// A program's rows, K-form's relations each at its head (`None`: ∅), each row its relation's columns (a relation a
/// recursive stratum writes carries its level after them).
fn program_rows(dir: &Directory, program: &[Option<Between<'_>>]) -> Result<Vec<BTreeSet<Vec<i64>>>, String> {
    program
        .iter()
        .zip(RELATIONS)
        .map(|(p, (_, arity))| match p {
            None => Ok(BTreeSet::new()),
            Some((t, _, h)) => Ok(rows_at(dir, t, *h)?.into_iter().map(|mut r| {
                r.truncate(arity);
                r
            }).collect()),
        })
        .collect()
}

/// K-form's relations' symbols, in `RELATIONS`' order (`None`: not interned, so no program names it).
pub fn kform_syms(dir: &Directory, atoms: Atoms) -> Vec<Option<i64>> {
    RELATIONS.iter().map(|(n, _)| atoms.find(dir, n)).collect()
}

/// The levels of B§8's loop, the first L2 (P2's), each run on the export of the one before.
pub struct Levels {
    pub ops: PhaseOps,
    pub levels: Vec<Level>,
}

impl Levels {
    pub fn new(ops: PhaseOps) -> Levels {
        Levels { ops, levels: Vec::new() }
    }

    /// Each level's pins.
    pub fn pins(&self) -> Vec<LevelPins> {
        self.levels.iter().map(Level::pins).collect()
    }

    /// The levels a kept run left at `pins`, placed at `place`: each one's program read again from where the level
    /// before it left its K-form (L1's outputs `l1` first).
    pub fn resume(&mut self, dir: &Directory, atoms: Atoms, place: u32, l1: &[Trace], pins: &[LevelPins]) -> Result<(), String> {
        let ksyms = kform_syms(dir, atoms);
        for p in pins {
            let rows = match self.levels.last() {
                None => program_rows(dir, &l1.iter().map(|t| Some((t, None, t.head()))).collect::<Vec<_>>())?,
                Some(prev) => program_rows(dir, &prev.program(&ksyms))?,
            };
            let ran = {
                let l = dir.read("atoms", &[atoms.0])?;
                Program::read(&rows, l.block(0)).map_err(|e| e.to_string())?
            };
            self.levels.push(Level::resumed(dir, place, p, ran)?);
        }
        Ok(())
    }

    /// The documents its levels read.
    pub fn docs(&self, dir: &Directory) -> Result<BTreeSet<i64>, String> {
        let mut all = BTreeSet::new();
        for l in &self.levels {
            all.extend(l.docs(dir)?);
        }
        Ok(all)
    }

    /// The levels from `k` on let go: the next run of them is from ∅.
    pub fn clear_from(&mut self, dir: &mut Directory, k: usize) {
        for l in self.levels.iter_mut().skip(k) {
            l.clear(dir);
        }
        self.levels.truncate(k);
    }

    /// Every level let go (a cold run's start).
    pub fn clear(&mut self, dir: &mut Directory) {
        self.levels.iter_mut().for_each(|l| l.clear(dir));
        self.levels.clear();
    }
}

/// The shape of the rows of the layer `head`, as the shape a stratum gives its head's trace.
fn rows_shape(dir: &Directory, head: BlockId) -> Result<Shape, String> {
    let l = dir.read("a trace's head", &[head])?;
    let w = rerail_blocks::layer::Layer::new(l.block(0))?.head.shape;
    let columns: Vec<ColType> = (0..w.arity()).map(|c| if w.width(c) == 8 { ColType::I64 } else { ColType::I32 }).collect();
    let mut s = Shape::run(columns, w.order().iter().map(|&c| u16::from(c)).collect(), w.partition().map(|p| p as u16));
    s.levels = w.level_col().is_some();
    Ok(s)
}

/// A trace that stands for one taken out while a stratum's instance runs on it.
fn placeholder() -> Trace {
    Trace::new(&Shape::run(vec![ColType::I64], vec![0], None)).expect("a shape").with_roll_up(RollUp::Never)
}

#[cfg(test)]
#[path = "phase_tests.rs"]
mod tests;
