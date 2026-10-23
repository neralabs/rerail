//! Generic rewrites over the algebra (INVARIANTS I4; registry `docs/facts/REWRITES.md` R6–R11).
//! Each takes `ir::Rel`/`ir::Plan`, never SQL text or a name, checks its side conditions in code
//! (from I2's declared structure where it needs a property of a relation), and returns the
//! rewritten algebra or `None`. The physical side (S2) executes the rewritten form; the
//! differential tests below compare both forms under the reference evaluator.
use crate::ir::*;
use crate::reference::{self, Row};
use crate::sql::value::{self as sv, Aff, V};
use std::collections::BTreeSet;

/// What a rewrite needs to know about a source: whether it is a stage of the plan (and which),
/// and whether it is a core relation of the content (I2's declared structure).
pub trait Source: Clone + std::fmt::Debug {
    fn stage(&self) -> Option<StageId>;
    fn as_stage(id: StageId) -> Self;
    fn core(&self) -> Option<CoreRel> {
        None
    }
}

impl Source for FileSource {
    fn stage(&self) -> Option<StageId> {
        if let FileSource::Stage(s) = self { Some(*s) } else { None }
    }
    fn as_stage(id: StageId) -> Self {
        FileSource::Stage(id)
    }
    fn core(&self) -> Option<CoreRel> {
        if let FileSource::Core(c) = self { Some(*c) } else { None }
    }
}

impl Source for RepoSource {
    fn stage(&self) -> Option<StageId> {
        if let RepoSource::Stage(s) = self { Some(*s) } else { None }
    }
    fn as_stage(id: StageId) -> Self {
        RepoSource::Stage(id)
    }
}

impl Source for AnySource {
    fn stage(&self) -> Option<StageId> {
        if let AnySource::Stage(s) = self { Some(*s) } else { None }
    }
    fn as_stage(id: StageId) -> Self {
        AnySource::Stage(id)
    }
}

fn width<S>(r: &Rel<S>) -> usize {
    reference::width(r)
}

fn col<S>(c: u16) -> Expr<S> {
    Expr::Col { depth: 0, col: c }
}

fn same<S: std::fmt::Debug>(a: &Rel<S>, b: &Rel<S>) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// Every relation of a plan: stage definitions, then the sink's.
fn plan_rels_mut<S>(p: &mut Plan<S>, from: usize, f: &mut dyn FnMut(&mut Rel<S>)) {
    for st in p.stages.iter_mut().skip(from) {
        match &mut st.def {
            StageDef::Query { rel } => f(rel),
            StageDef::Reach { roots, edges, .. } => {
                f(roots);
                if let Some(e) = edges {
                    f(e)
                }
            }
        }
    }
    match &mut p.sink {
        Sink::Findings { rel, .. } | Sink::Budget { rel, .. } => f(rel),
        Sink::Export(list) => list.iter_mut().for_each(|(_, r)| f(r)),
        Sink::Witnesses { .. } => {}
    }
}

/// Visits every relation node (subqueries included), pre-order, mutably.
fn walk_mut<S>(r: &mut Rel<S>, f: &mut dyn FnMut(&mut Rel<S>)) {
    f(r);
    children_mut(r, &mut |c| walk_mut(c, f));
}

fn count_stage_reads<S: Source>(p: &Plan<S>) -> Vec<usize> {
    let mut n = vec![0; p.stages.len()];
    let mut p = p.clone();
    plan_rels_mut(&mut p, 0, &mut |r| {
        walk_mut(r, &mut |x| {
            if let Rel::Scan { src, .. } = x {
                if let Some(s) = src.stage() {
                    n[s as usize] += 1;
                }
            }
        })
    });
    n
}

/// Removes stages `dead` (unread) and renumbers the remaining stage scans.
fn drop_stages<S: Source>(mut p: Plan<S>, dead: &BTreeSet<StageId>) -> Plan<S> {
    if dead.is_empty() {
        return p;
    }
    let map: Vec<Option<StageId>> = {
        let mut k = 0;
        (0..p.stages.len() as StageId).map(|i| if dead.contains(&i) { None } else { k += 1; Some(k - 1) }).collect()
    };
    plan_rels_mut(&mut p, 0, &mut |r| {
        walk_mut(r, &mut |x| {
            if let Rel::Scan { src, .. } = x {
                if let Some(s) = src.stage() {
                    *src = S::as_stage(map[s as usize].expect("a dropped stage is read"));
                }
            }
        })
    });
    p.stages = p.stages.into_iter().enumerate().filter(|(i, _)| !dead.contains(&(*i as StageId))).map(|(_, s)| s).collect();
    p
}

// ---------------------------------------------------------------------------------------------
// R6 view inlining

/// Whether every value of column `c` of `r` already has affinity `aff` applied, so storing it
/// in a stage column of that affinity (CTAS) changes nothing. Stage columns are stored with
/// their affinity; `CAST(x AS aff)` yields aff-normal values; a literal is checked.
fn affinity_normal<S: Source>(r: &Rel<S>, c: u16, aff: Aff, stages: &[Stage<S>]) -> bool {
    if aff == Aff::None || crate::mutant::on(16) {
        // Mutant 16 (dropped side condition): every column taken as affinity-normal.
        return true;
    }
    match r {
        Rel::Scan { src, schema } => src.stage().is_some() && schema.get(c as usize).is_some_and(|k| k.aff == aff),
        Rel::Project { input, exprs, .. } => match &exprs[c as usize] {
            Expr::Col { depth: 0, col } => affinity_normal(input, *col, aff, stages),
            Expr::Cast(_, a) => *a == aff,
            Expr::Lit(l) => {
                let v = reference::lit(l);
                format!("{:?}", sv::apply_affinity(v.clone(), aff)) == format!("{v:?}")
            }
            _ => false,
        },
        Rel::Filter { input, .. } | Rel::Sort { input, .. } | Rel::Distinct { input, .. } => affinity_normal(input, c, aff, stages),
        Rel::Join { left, right, .. } => {
            let wl = width(left) as u16;
            if c < wl { affinity_normal(left, c, aff, stages) } else { affinity_normal(right, c - wl, aff, stages) }
        }
        Rel::Union { left, right, .. } => affinity_normal(left, c, aff, stages) && affinity_normal(right, c, aff, stages),
        Rel::JsonEach { input, .. } | Rel::Aggregate { input, .. } | Rel::Window { input, .. } => (c as usize) < width(input) && affinity_normal(input, c, aff, stages),
        Rel::With { body, .. } => affinity_normal(body, c, aff, stages),
        Rel::Local { .. } | Rel::Unit => false,
    }
}

/// R6: a `Query` stage read exactly once (by a later stage or the sink) is replaced, at its
/// read, by its relation; the stage is dropped. Side conditions: one read (more reads would
/// recompute it; equal digests are CSE's business); its stored values are affinity-normal for
/// its declared column affinities (storing applies them, inlining does not). Returns the
/// inlined stage names.
pub fn inline_views<S: Source>(mut p: Plan<S>) -> (Plan<S>, Vec<String>) {
    let reads = count_stage_reads(&p);
    let mut dead = BTreeSet::new();
    let mut names = Vec::new();
    for s in 0..p.stages.len() {
        let StageDef::Query { rel } = &p.stages[s].def else { continue };
        if reads[s] != 1 || !p.stages[s].schema.iter().enumerate().all(|(c, k)| affinity_normal(rel, c as u16, k.aff, &p.stages)) {
            continue;
        }
        let rel = rel.clone();
        let id = s as StageId;
        plan_rels_mut(&mut p, s + 1, &mut |r| {
            walk_mut(r, &mut |x| {
                if matches!(x, Rel::Scan { src, .. } if src.stage() == Some(id)) {
                    *x = rel.clone();
                }
            })
        });
        dead.insert(id);
        names.push(p.stages[s].name.clone());
    }
    (drop_stages(p, &dead), names)
}

// ---------------------------------------------------------------------------------------------
// R7 union-of-selections fusion

/// One arm of a fused union: an optional selection and the arm's projection over the base.
#[derive(Clone, Debug)]
pub struct Arm<S> {
    pub pred: Option<Expr<S>>,
    pub exprs: Vec<Expr<S>>,
}

/// `UNION ALL` of k selections of one relation: one pass over `base` evaluating every arm's
/// selection per row, the output in arm order, base order within an arm.
#[derive(Clone, Debug)]
pub struct Fusion<S> {
    pub base: Rel<S>,
    pub arms: Vec<Arm<S>>,
    pub schema: Vec<Column>,
}

fn union_arms<'r, S>(r: &'r Rel<S>, out: &mut Vec<&'r Rel<S>>) {
    match r {
        Rel::Union { all: true, left, right } => {
            union_arms(left, out);
            union_arms(right, out);
        }
        o => out.push(o),
    }
}

/// R7: recognises `UNION ALL` of ≥ 2 arms `[Project] [Filter…] X` over one relation `X`
/// (structurally equal, compared as algebra). Side condition: every arm's base is the same
/// relation. Selections stack as conjunctions in their evaluation order.
pub fn fuse_union<S: Source>(r: &Rel<S>) -> Option<Fusion<S>> {
    let mut arms = Vec::new();
    union_arms(r, &mut arms);
    if arms.len() < 2 {
        return None;
    }
    let mut out = Vec::new();
    let mut base: Option<&Rel<S>> = None;
    for a in arms {
        let (exprs, mut cur) = match a {
            Rel::Project { input, exprs, .. } => (Some(exprs.clone()), &**input),
            o => (None, o),
        };
        let mut preds = Vec::new();
        while let Rel::Filter { input, pred } = cur {
            preds.push(pred.clone());
            cur = input;
        }
        preds.reverse();
        match base {
            None => base = Some(cur),
            // Mutant 17 (dropped side condition): arms over different relations are fused.
            Some(b) if !same(b, cur) && !crate::mutant::on(17) => return None,
            Some(_) => {}
        }
        let w = width(cur) as u16;
        out.push(Arm { pred: conjoin(preds), exprs: exprs.unwrap_or_else(|| (0..w).map(col).collect()) });
    }
    let base = base.unwrap().clone();
    if out.iter().any(|a| a.exprs.len() != out[0].exprs.len()) {
        return None;
    }
    Some(Fusion { base, arms: out, schema: r.schema() })
}

impl<S: Source> Fusion<S> {
    /// The fused form in the algebra: `base ⋈ arms(tag)` (one pass over `base`, each row tried
    /// against every arm), each arm's selection and projection chosen by the tag, then a stable
    /// sort on the tag (arm order, base order within an arm) and the tag dropped.
    pub fn to_rel(&self) -> Rel<S> {
        let w = width(&self.base) as u16;
        let tags = self
            .arms
            .iter()
            .enumerate()
            .map(|(k, _)| Rel::Project { input: Box::new(Rel::Unit), exprs: vec![Expr::Lit(Lit::Int(k as i64))], schema: vec![Column::new("__arm", Aff::None)] })
            .reduce(|a, b| Rel::Union { all: true, left: Box::new(a), right: Box::new(b) })
            .unwrap();
        let tag = || Box::new(col::<S>(w));
        let by_tag = |pick: &dyn Fn(&Arm<S>) -> Expr<S>| Expr::Case {
            operand: Some(tag()),
            whens: self.arms.iter().enumerate().map(|(k, a)| (Expr::Lit(Lit::Int(k as i64)), pick(a), (Aff::None, Aff::None))).collect(),
            else_: None,
        };
        let on = by_tag(&|a| a.pred.clone().unwrap_or(Expr::Lit(Lit::Int(1))));
        let joined = Rel::Join { kind: JoinKind::Inner, left: Box::new(self.base.clone()), right: Box::new(tags), on: Some(on) };
        let n = self.arms[0].exprs.len();
        let mut exprs = vec![col(w)];
        let mut schema = vec![Column::new("__arm", Aff::None)];
        for c in 0..n {
            exprs.push(by_tag(&|a| a.exprs[c].clone()));
            schema.push(self.schema[c].clone());
        }
        let projected = Rel::Project { input: Box::new(joined), exprs, schema };
        let sorted = Rel::Sort { input: Box::new(projected), keys: vec![(0, false)], limit: None, offset: None, total: Total::InputOrder };
        Rel::Project { input: Box::new(sorted), exprs: (1..=n as u16).map(col).collect(), schema: self.schema.clone() }
    }
}

// ---------------------------------------------------------------------------------------------
// R8 projection pruning

/// Columns of each stage that a relation reads, given the columns of `r` its consumer reads.
fn demand<S: Source>(r: &Rel<S>, needed: &BTreeSet<u16>, reads: &mut Vec<BTreeSet<u16>>) {
    let all = |w: usize| (0..w as u16).collect::<BTreeSet<u16>>();
    let read = |e: &Expr<S>, out: &mut BTreeSet<u16>| cols_read(e, &mut |c| {
        out.insert(c);
    });
    // Subqueries in an operator's expressions: existence reads no column, IN and a scalar the
    // first; their correlations are reads of the operator's row (`cols_read` follows them).
    let subs = |e: &Expr<S>, reads: &mut Vec<BTreeSet<u16>>| {
        fn go<S: Source>(e: &Expr<S>, reads: &mut Vec<BTreeSet<u16>>) {
            each_subexpr(e, &mut |x| match x {
                Expr::Exists { rel, .. } => demand(rel, &BTreeSet::new(), reads),
                Expr::InRel { rel, .. } | Expr::Scalar(rel) => demand(rel, &BTreeSet::from([0]), reads),
                _ => {}
            });
        }
        go(e, reads)
    };
    match r {
        Rel::Scan { src, .. } => {
            if let Some(s) = src.stage() {
                reads[s as usize].extend(needed.iter().copied());
            }
        }
        Rel::Local { .. } | Rel::Unit => {}
        Rel::JsonEach { input, arg, on, .. } => {
            let wi = width(input) as u16;
            let mut n: BTreeSet<u16> = needed.iter().copied().filter(|c| *c < wi).collect();
            read(arg, &mut n);
            subs(arg, reads);
            if let Some(o) = on {
                read(o, &mut n);
                subs(o, reads);
            }
            n.retain(|c| *c < wi);
            demand(input, &n, reads);
        }
        Rel::Join { left, right, on, .. } => {
            let wl = width(left) as u16;
            let mut n = needed.clone();
            if let Some(o) = on {
                read(o, &mut n);
                subs(o, reads);
            }
            demand(left, &n.iter().copied().filter(|c| *c < wl).collect(), reads);
            demand(right, &n.iter().filter(|c| **c >= wl).map(|c| c - wl).collect(), reads);
        }
        Rel::Filter { input, pred } => {
            let mut n = needed.clone();
            read(pred, &mut n);
            subs(pred, reads);
            demand(input, &n, reads);
        }
        Rel::Project { input, exprs, .. } => {
            let mut n = BTreeSet::new();
            for (j, e) in exprs.iter().enumerate() {
                if needed.contains(&(j as u16)) {
                    read(e, &mut n);
                    subs(e, reads);
                }
            }
            demand(input, &n, reads);
        }
        Rel::Aggregate { input, keys, aggs } => {
            let wi = width(input) as u16;
            // Representative-row columns are read where the consumer reads them.
            let mut n: BTreeSet<u16> = needed.iter().copied().filter(|c| *c < wi).collect();
            for e in keys.iter().chain(aggs.iter().flat_map(|a| a.arg.iter().chain(a.sep.iter()))) {
                read(e, &mut n);
                subs(e, reads);
            }
            demand(input, &n, reads);
        }
        Rel::Window { input, funcs } => {
            let wi = width(input) as u16;
            let mut n: BTreeSet<u16> = needed.iter().copied().filter(|c| *c < wi).collect();
            for f in funcs {
                for e in f.partition.iter().chain(f.order.iter().map(|o| &o.0)) {
                    read(e, &mut n);
                }
                if let WindowKind::Min(e) | WindowKind::Max(e) = &f.kind {
                    read(e, &mut n);
                }
            }
            demand(input, &n, reads);
        }
        Rel::Distinct { input, prefix } => {
            let mut n = needed.clone();
            // The rows kept depend on every prefix column.
            if !crate::mutant::on(18) {
                // Mutant 18 (dropped side condition): DISTINCT's prefix is not a read.
                n.extend(0..*prefix);
            }
            demand(input, &n, reads);
        }
        Rel::Union { all, left, right } => {
            let n = if *all { needed.clone() } else { all_cols(width(left)) };
            demand(left, &n, reads);
            demand(right, &n, reads);
        }
        Rel::Sort { input, keys, limit, offset, .. } => {
            let mut n = needed.clone();
            n.extend(keys.iter().map(|k| k.0));
            for e in limit.iter().chain(offset) {
                subs(e, reads);
            }
            demand(input, &n, reads);
        }
        Rel::With { bindings, body } => {
            // Bindings are read whole (conservative).
            for (_, b) in bindings {
                match b {
                    Binding::Plain(r) => demand(r, &all(width(r)), reads),
                    Binding::Fix { init, step, schema, .. } => init.iter().chain(step).for_each(|r| demand(r, &all(schema.len()), reads)),
                }
            }
            demand(body, needed, reads);
        }
    }
}

fn all_cols(w: usize) -> BTreeSet<u16> {
    (0..w as u16).collect()
}

/// R8: a `Query` stage column no reader reads is not computed: the stage is narrowed to its
/// read columns (`Project` on top), and each read re-widens it with `NULL` at unread positions,
/// so readers' column numbers are unchanged. A column is read when a reader's output needs it,
/// or a predicate, join key, sort key, group key, aggregate argument, window key, `DISTINCT`
/// prefix or set `UNION` reads it, or a correlated subquery reads it. Sinks read every column.
/// R4 (export projection) is this rewrite at the export boundary. Returns `(stage, kept, of)`.
pub fn prune_columns<S: Source>(mut p: Plan<S>) -> (Plan<S>, Vec<(String, usize, usize)>) {
    let n = p.stages.len();
    let mut reads: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); n];
    match &p.sink {
        Sink::Findings { rel, .. } | Sink::Budget { rel, .. } => demand(rel, &all_cols(width(rel)), &mut reads),
        Sink::Export(list) => list.iter().for_each(|(_, r)| demand(r, &all_cols(width(r)), &mut reads)),
        Sink::Witnesses { .. } => {}
    }
    for s in (0..n).rev() {
        match &p.stages[s].def {
            StageDef::Query { rel } => {
                let need = reads[s].clone();
                demand(rel, &need, &mut reads);
            }
            StageDef::Reach { roots, edges, .. } => {
                demand(roots, &all_cols(width(roots)), &mut reads);
                if let Some(e) = edges {
                    demand(e, &all_cols(width(e)), &mut reads);
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in 0..n {
        let w = p.stages[s].schema.len();
        let keep: Vec<u16> = reads[s].iter().copied().collect();
        // An unread stage keeps one column: it is not removed here (dead-stage elimination is
        // not this rewrite), only narrowed.
        if keep.len() == w || !matches!(p.stages[s].def, StageDef::Query { .. }) || keep.is_empty() {
            continue;
        }
        let wide = p.stages[s].schema.clone();
        let narrow: Vec<Column> = keep.iter().map(|c| wide[*c as usize].clone()).collect();
        if let StageDef::Query { rel } = &mut p.stages[s].def {
            let inner = std::mem::replace(rel, Rel::Unit);
            *rel = Rel::Project { input: Box::new(inner), exprs: keep.iter().map(|c| col(*c)).collect(), schema: narrow.clone() };
        }
        p.stages[s].schema = narrow.clone();
        let id = s as StageId;
        let rewiden: Vec<Expr<S>> = (0..w as u16).map(|c| match keep.iter().position(|k| *k == c) {
            Some(j) => col(j as u16),
            None => Expr::Lit(Lit::Null),
        }).collect();
        plan_rels_mut(&mut p, s + 1, &mut |r| {
            walk_mut(r, &mut |x| {
                if let Rel::Scan { src, schema } = x {
                    // The re-widened scan is narrow: not replaced again.
                    if src.stage() == Some(id) && schema.len() == w {
                        let scan = Rel::Scan { src: src.clone(), schema: narrow.clone() };
                        *x = Rel::Project { input: Box::new(scan), exprs: rewiden.clone(), schema: wide.clone() };
                    }
                }
            })
        });
        out.push((p.stages[s].name.clone(), keep.len(), w));
    }
    (p, out)
}

// ---------------------------------------------------------------------------------------------
// Origins: which core column a relation's column carries (I2's declared structure)

/// A relation's column traced to a column of one core scan: the scan's identity (its address
/// in the plan), the relation and the column name. Through inner joins, filters, projections of
/// a column, sorts and DISTINCT only (a left join's right side may be NULL-extended).
fn origin<S: Source>(r: &Rel<S>, c: u16) -> Option<(usize, CoreRel, &'static str)> {
    match r {
        Rel::Scan { src, .. } => src.core().map(|k| (r as *const Rel<S> as usize, k, k.columns()[c as usize])),
        Rel::Project { input, exprs, .. } => match exprs.get(c as usize)? {
            Expr::Col { depth: 0, col } => origin(input, *col),
            _ => None,
        },
        Rel::Filter { input, .. } | Rel::Sort { input, .. } | Rel::Distinct { input, .. } => origin(input, c),
        Rel::Join { kind: JoinKind::Inner, left, right, .. } => {
            let wl = width(left) as u16;
            if c < wl { origin(left, c) } else { origin(right, c - wl) }
        }
        _ => None,
    }
}

/// Whether `r` holds every row of one `node` scan (only `file` joined to it, filters only
/// `parent IS NOT NULL`), returning the scan's identity: `r` then holds every tree edge.
fn all_node_rows<S: Source>(r: &Rel<S>, parent_col: u16) -> Option<usize> {
    match r {
        Rel::Scan { src, .. } if src.core() == Some(CoreRel::Node) => Some(r as *const Rel<S> as usize),
        Rel::Project { input, exprs, .. } => {
            let Expr::Col { depth: 0, col } = exprs.get(parent_col as usize)? else { return None };
            all_node_rows(input, *col)
        }
        Rel::Filter { input, pred } => {
            let ok = conjuncts(pred).iter().all(|c| matches!(c, Expr::IsNull { e, negated: true } if matches!(**e, Expr::Col { depth: 0, col } if col == parent_col)));
            if ok { all_node_rows(input, parent_col) } else { None }
        }
        Rel::Join { kind: JoinKind::Inner, left, right, on: None } => {
            let wl = width(left) as u16;
            let file = |x: &Rel<S>| matches!(x, Rel::Scan { src, .. } if src.core() == Some(CoreRel::File));
            if file(left) && parent_col >= wl {
                all_node_rows(right, parent_col - wl)
            } else if file(right) && parent_col < wl {
                all_node_rows(left, parent_col)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Order observation

/// Whether the row order of stage `s` can change the plan's result. Findings are a set of
/// findings (identity key), so the sink observes no order; order reaches a result only through
/// `row_number()`, `LIMIT`/`OFFSET`, a stable sort's ties, ordered aggregates, aggregate bare
/// columns, first-occurrence `DISTINCT` over a prefix, a scalar subquery's first row, a
/// fixpoint, or an export read in order (conservatively: every export).
pub fn order_observed<S: Source>(p: &Plan<S>, s: StageId) -> bool {
    let n = p.stages.len();
    // Per stage: whether its order is observed; computed from the readers, last first.
    let mut observed = vec![false; n];
    fn walk<S: Source>(r: &Rel<S>, om: bool, out: &mut Vec<bool>) {
        let sub = |e: &Expr<S>, out: &mut Vec<bool>| {
            each_subexpr(e, &mut |x| match x {
                Expr::Exists { rel, .. } | Expr::InRel { rel, .. } => walk(rel, false, out),
                Expr::Scalar(rel) => walk(rel, true, out),
                _ => {}
            })
        };
        match r {
            Rel::Scan { src, .. } => {
                if let (Some(t), true) = (src.stage(), om) {
                    out[t as usize] = true;
                }
            }
            Rel::Local { .. } | Rel::Unit => {}
            Rel::Filter { input, pred } => {
                sub(pred, out);
                walk(input, om, out)
            }
            Rel::Project { input, exprs, .. } => {
                exprs.iter().for_each(|e| sub(e, out));
                walk(input, om, out)
            }
            Rel::JsonEach { input, arg, on, .. } => {
                sub(arg, out);
                on.iter().for_each(|e| sub(e, out));
                walk(input, om, out)
            }
            Rel::Join { left, right, on, .. } => {
                on.iter().for_each(|e| sub(e, out));
                walk(left, om, out);
                walk(right, om, out)
            }
            Rel::Union { left, right, .. } => {
                walk(left, om, out);
                walk(right, om, out)
            }
            Rel::Distinct { input, prefix } => walk(input, om || (*prefix as usize) < width(input), out),
            Rel::Sort { input, limit, offset, .. } => walk(input, om || limit.is_some() || offset.is_some(), out),
            Rel::Aggregate { input, .. } => walk(input, true, out),
            Rel::Window { input, funcs } => walk(input, om || funcs.iter().any(|f| matches!(f.kind, WindowKind::RowNumber)), out),
            Rel::With { bindings, body } => {
                for (_, b) in bindings {
                    match b {
                        Binding::Plain(r) => walk(r, true, out),
                        Binding::Fix { init, step, .. } => init.iter().chain(step).for_each(|r| walk(r, true, out)),
                    }
                }
                walk(body, om, out)
            }
        }
    }
    match &p.sink {
        Sink::Findings { rel, .. } => walk(rel, false, &mut observed),
        Sink::Budget { rel, .. } => walk(rel, true, &mut observed),
        Sink::Export(list) => list.iter().for_each(|(_, r)| walk(r, true, &mut observed)),
        Sink::Witnesses { .. } => {}
    }
    for t in (0..n).rev() {
        let om = observed[t];
        match &p.stages[t].def {
            StageDef::Query { rel } => walk(rel, om, &mut observed),
            StageDef::Reach { roots, edges, .. } => {
                walk(roots, true, &mut observed);
                if let Some(e) = edges {
                    walk(e, true, &mut observed)
                }
            }
        }
    }
    observed[s as usize]
}

// ---------------------------------------------------------------------------------------------
// R9 tree reachability → interval

/// R9: a stage computing reachability over the tree edge by a set fixpoint
/// `walk(p, root, node) := {(p, id, id) | R} ∪ {(p, w.root, e.dst) | walk w ⋈ E e on e.p = w.p ∧
/// e.src = w.node}` becomes an interval join on the declared preorder (I2: `d` is a descendant
/// of `x` iff `x < d < subtree_end(x)`): `DISTINCT (p, r.id, c.id) FROM R ⋈ node r2 ON r2.id =
/// R.id ⋈ node c ON r2.id ≤ c.id < r2.subtree_end`. Side conditions, checked: `E` holds every
/// row of one `node` scan with `src` = its `parent` and `dst` = its `id` (the tree's
/// parent/child relation); the roots' `id` is a core `node.id` (non-NULL, preorder); both path
/// columns are `file.path`; the stage's row order is not observed (the fixpoint emits level
/// order, the interval join root then preorder). Returns the rewritten stage names.
pub fn reach_to_interval(mut p: Plan<FileSource>) -> (Plan<FileSource>, Vec<String>) {
    let mut done = Vec::new();
    for s in 0..p.stages.len() {
        let StageDef::Query { rel } = &p.stages[s].def else { continue };
        let Some(new) = interval_form(rel) else { continue };
        // Mutant 19 (dropped side condition): the stage's order is taken as unobserved.
        if order_observed(&p, s as StageId) && !crate::mutant::on(19) {
            continue;
        }
        p.stages[s].def = StageDef::Query { rel: new };
        done.push(p.stages[s].name.clone());
    }
    (p, done)
}

fn interval_form(rel: &Rel<FileSource>) -> Option<Rel<FileSource>> {
    let Rel::With { bindings, body } = rel else { return None };
    let [(l, Binding::Fix { init, step, set: true, schema })] = bindings.as_slice() else { return None };
    if schema.len() != 3 || init.len() != 1 || step.len() != 1 {
        return None;
    }
    let body_ok = match &**body {
        Rel::Local { id, .. } => id == l,
        Rel::Project { input, exprs, .. } => matches!(&**input, Rel::Local { id, .. } if id == l) && exprs.len() == 3 && exprs.iter().enumerate().all(|(i, e)| matches!(e, Expr::Col { depth: 0, col } if *col as usize == i)),
        _ => false,
    };
    if !body_ok {
        return None;
    }
    // Seeds: (p, x, x) over R.
    let Rel::Project { input: roots, exprs: ie, .. } = &init[0] else { return None };
    let (Expr::Col { depth: 0, col: rp }, Expr::Col { depth: 0, col: rx }, Expr::Col { depth: 0, col: rx2 }) = (&ie[0], &ie[1], &ie[2]) else { return None };
    if rx != rx2 || origin(roots, *rx).map(|o| (o.1, o.2)) != Some((CoreRel::Node, "id")) || origin(roots, *rp).map(|o| (o.1, o.2)) != Some((CoreRel::File, "path")) {
        return None;
    }
    // Step: (w.p, w.root, e.dst) over walk ⋈ E on e.p = w.p ∧ e.src = w.node.
    let Rel::Project { input: j, exprs: se, .. } = &step[0] else { return None };
    let Rel::Join { kind: JoinKind::Inner, left, right: e, on: Some(on) } = &**j else { return None };
    if !matches!(&**left, Rel::Local { id, .. } if id == l) {
        return None;
    }
    let (Expr::Col { depth: 0, col: 0 }, Expr::Col { depth: 0, col: 1 }, Expr::Col { depth: 0, col: dst }) = (&se[0], &se[1], &se[2]) else { return None };
    let dst = dst.checked_sub(3)?;
    let (lk, rk, rest) = equi_keys(Some(on), 3);
    if rest.is_some() || lk.len() != 2 {
        return None;
    }
    let mut e_path = None;
    let mut e_src = None;
    for (a, b) in lk.iter().zip(&rk) {
        match (&a.e, &b.e) {
            (Expr::Col { depth: 0, col: 0 }, Expr::Col { depth: 0, col }) => e_path = Some(*col),
            (Expr::Col { depth: 0, col: 2 }, Expr::Col { depth: 0, col }) => e_src = Some(*col),
            _ => return None,
        }
    }
    let (e_path, e_src) = (e_path?, e_src?);
    let (so, dso) = (origin(e, e_src)?, origin(e, dst)?);
    if (so.1, so.2, dso.1, dso.2) != (CoreRel::Node, "parent", CoreRel::Node, "id") || so.0 != dso.0 || origin(e, e_path).map(|o| (o.1, o.2)) != Some((CoreRel::File, "path")) {
        return None;
    }
    if all_node_rows(e, e_src)? != so.0 {
        return None;
    }
    // R ⋈ node r2 ON r2.id = R.x ⋈ node c ON r2.id <= c.id AND c.id < r2.subtree_end.
    let node = || Rel::Scan { src: FileSource::Core(CoreRel::Node), schema: CoreRel::Node.schema() };
    let wr = width(roots) as u16;
    let cmp = |op, a: u16, b: u16| Expr::Cmp(op, Box::new(col(a)), Box::new(col(b)), (Aff::None, Aff::None));
    let r2 = Rel::Join { kind: JoinKind::Inner, left: roots.clone(), right: Box::new(node()), on: Some(cmp(CmpOp::Eq, wr, *rx)) };
    let n = CoreRel::Node.columns().len() as u16;
    let (r2_id, r2_end, c_id) = (wr, wr + 7, wr + n);
    let joined = Rel::Join { kind: JoinKind::Inner, left: Box::new(r2), right: Box::new(node()), on: Some(Expr::And(Box::new(cmp(CmpOp::Ge, c_id, r2_id)), Box::new(cmp(CmpOp::Lt, c_id, r2_end)))) };
    Some(Rel::Distinct { input: Box::new(Rel::Project { input: Box::new(joined), exprs: vec![col(*rp), col(*rx), col(c_id)], schema: schema.clone() }), prefix: 3 })
}

// ---------------------------------------------------------------------------------------------
// R10 disjunctive equality → union of lookups

/// A join whose `ON` holds a disjunction of equality groups: the right rows for a left row are
/// the union of one key lookup per disjunct, deduplicated, in right-table order, and the whole
/// `ON` is re-checked on each candidate.
#[derive(Clone, Debug)]
pub struct OrProbe<S> {
    /// Per disjunct: its equality keys (left, right side; right rebased).
    pub branches: Vec<(Vec<Key<S>>, Vec<Key<S>>)>,
    pub on: Expr<S>,
    pub left_width: usize,
}

/// R10: recognises the first `ON` conjunct that is an `OR` of disjuncts each holding at least
/// one equality between a left and a right expression. Side condition: every disjunct has a key
/// (a disjunct without one would need a scan of the right side).
pub fn or_probe<S: Clone>(on: &Expr<S>, left_width: usize) -> Option<OrProbe<S>> {
    fn disjuncts<S: Clone>(e: &Expr<S>, out: &mut Vec<Expr<S>>) {
        match e {
            Expr::Or(a, b) => {
                disjuncts(a, out);
                disjuncts(b, out)
            }
            o => out.push(o.clone()),
        }
    }
    for c in conjuncts(on) {
        let mut ds = Vec::new();
        disjuncts(&c, &mut ds);
        if ds.len() < 2 {
            continue;
        }
        let mut branches = Vec::new();
        for d in &ds {
            let (l, r, _) = equi_keys(Some(d), left_width);
            if l.is_empty() {
                branches.clear();
                break;
            }
            branches.push((l, r));
        }
        if branches.len() == ds.len() {
            return Some(OrProbe { branches, on: on.clone(), left_width });
        }
    }
    None
}

impl<S: Clone> OrProbe<S> {
    /// The reference form of the physical probe: the right rows whose keys equal the left row's
    /// for some disjunct (SQL `=`: NULL matches nothing), sorted and deduplicated (table order),
    /// then filtered by the whole `ON`.
    pub fn matches(&self, left: &[V], right: &[Row]) -> Result<Vec<usize>, String> {
        let mut cand = Vec::new();
        for (lk, rk) in &self.branches {
            let lv: Vec<V> = lk.iter().map(|k| reference::eval_scalar(&k.e, left, &[])).collect::<Result<_, _>>()?;
            for (j, row) in right.iter().enumerate() {
                let mut hit = true;
                for ((l, r), v) in lk.iter().zip(rk).zip(&lv) {
                    let rv = reference::eval_scalar(&r.e, row, &[])?;
                    let eq = Expr::<S>::Cmp(CmpOp::Eq, Box::new(Expr::Lit(lit_of(v))), Box::new(Expr::Lit(lit_of(&rv))), l.affs);
                    if reference::truth(&reference::eval_scalar(&eq, &[], &[])?) != Some(true) {
                        hit = false;
                        break;
                    }
                }
                if hit {
                    cand.push(j);
                }
            }
        }
        cand.sort_unstable();
        // Mutant 20 (dropped side condition): a right row matching two disjuncts is not
        // deduplicated.
        if !crate::mutant::on(20) {
            cand.dedup();
        }
        let mut out = Vec::new();
        for j in cand {
            let mut row = left.to_vec();
            row.extend(right[j].iter().cloned());
            if reference::truth(&reference::eval_scalar(&self.on, &row, &[])?) == Some(true) {
                out.push(j);
            }
        }
        Ok(out)
    }
}

fn lit_of(v: &V) -> Lit {
    match v {
        V::Null => Lit::Null,
        V::Int(i) => Lit::Int(*i),
        V::Real(f) => Lit::Real(*f),
        V::Text(s) => Lit::Text((**s).into()),
    }
}

// ---------------------------------------------------------------------------------------------
// R11 selection sharing by semijoin under subtree closure

/// Positions of `node.id` and `node.subtree_end` of one node scan in a node-row relation.
fn node_cols<S: Source>(x: &Rel<S>) -> Option<(u16, u16)> {
    let w = width(x) as u16;
    let id = (0..w).find(|c| origin(x, *c).is_some_and(|o| (o.1, o.2) == (CoreRel::Node, "id")))?;
    let end = (0..w).find(|c| origin(x, *c).is_some_and(|o| (o.1, o.2) == (CoreRel::Node, "subtree_end")))?;
    (origin(x, id)?.0 == origin(x, end)?.0).then_some((id, end))
}

/// Whether a selection predicate over the rows of `x` (one row per node) selects a set closed
/// under descendants: constant in the node; a conjunction or disjunction of closed predicates;
/// `∃ a ancestor-or-self: Q(a)`; or `Q(n) ∨ ∃ a proper ancestor: Q(a)` (the pattern front end's
/// `any[X, inside{X, stopBy: end}]`), with `Q` over the candidate row only.
pub fn subtree_closed<S: Source>(pred: &Expr<S>, x: &Rel<S>) -> bool {
    // Mutant 21 (dropped side condition): every selection taken as subtree-closed.
    if crate::mutant::on(21) {
        return true;
    }
    let Some((id, _)) = node_cols(x) else { return false };
    let mut own = false;
    cols_read(pred, &mut |_| own = true);
    if !own {
        return true;
    }
    // An ancestor search: Exists(Filter(x', interval ∧ Q)) with Q over the candidate only.
    let ancestors = |e: &Expr<S>, or_self: bool| -> Option<Expr<S>> {
        let Expr::Exists { rel, negated: false } = e else { return None };
        let Rel::Filter { input, pred } = &**rel else { return None };
        let (cid, cend) = node_cols(input)?;
        let mut interval = (false, false);
        let mut q = Vec::new();
        for c in conjuncts(pred) {
            let is = |op: CmpOp, a: u16, b: u16| matches!(&c, Expr::Cmp(o, l, r, _) if *o == op && matches!(**l, Expr::Col { depth: 0, col } if col == a) && matches!(**r, Expr::Col { depth: 1, col } if col == b));
            if is(if or_self { CmpOp::Le } else { CmpOp::Lt }, cid, id) {
                interval.0 = true;
            } else if is(CmpOp::Gt, cend, id) {
                interval.1 = true;
            } else if reads_outer(&c, 1) {
                return None;
            } else {
                q.push(c);
            }
        }
        (interval == (true, true) && format!("{input:?}") == format!("{x:?}")).then(|| conjoin(q).unwrap_or(Expr::Lit(Lit::Int(1))))
    };
    match pred {
        Expr::And(a, b) => subtree_closed(a, x) && subtree_closed(b, x),
        Expr::Or(a, b) => {
            if let Some(q) = ancestors(b, false) {
                if format!("{q:?}") == format!("{a:?}") {
                    return true;
                }
            }
            subtree_closed(a, x) && subtree_closed(b, x)
        }
        e => ancestors(e, true).is_some(),
    }
}

/// R11: a stage `v` that reads the selection stage `sel = Filter(X, p)` (X one row per node)
/// as its anchor (left-most leaf) and reaches every other read of `sel` from an earlier one along
/// tree edges downward (`c.parent = x.id`, or `x.id < c.id < x.subtree_end`) computes, over the
/// selection, exactly the anchor-restricted rows of the same stage over all nodes, when `p` is
/// subtree-closed: `v[sel → X] ⋉ (anchor.id ∈ sel)` (semijoin at the anchor). The `v[sel → X]`
/// part is the same relation for every clause that differs only in its selection, so it is
/// computed once per content (sharing) and each clause filters it. Side conditions: `p`
/// subtree-closed ([`subtree_closed`]); `v` is joins/filters/projections over `sel` leaves (and
/// constants) with downward tree-edge joins only.
pub fn share_selection<S: Source>(v: &Rel<S>, sel: StageId, sel_def: &Rel<S>) -> Option<Rel<S>> {
    let Rel::Filter { input: x, pred } = sel_def else { return None };
    if !subtree_closed(pred, x) {
        return None;
    }
    let (xid, xend) = node_cols(x)?;
    let xw = width(x) as u16;
    // Leaves of v in join order with their column offsets; every leaf must be a `sel` scan.
    fn leaves<S: Source>(r: &Rel<S>, off: u16, sel: StageId, out: &mut Vec<u16>, conds: &mut Vec<Expr<S>>) -> bool {
        match r {
            Rel::Scan { src, .. } if src.stage() == Some(sel) => {
                out.push(off);
                true
            }
            Rel::Join { kind: JoinKind::Inner, left, right, on } => {
                if let Some(o) = on {
                    conds.extend(conjuncts(o).into_iter().map(|c| map_cols(c, &mut |_, k| k + off)));
                }
                leaves(left, off, sel, out, conds) && leaves(right, off + width(left) as u16, sel, out, conds)
            }
            _ => false,
        }
    }
    // The SPJ core: Project/Filter on top of the join tree.
    fn core<S: Source>(r: &Rel<S>) -> &Rel<S> {
        match r {
            Rel::Project { input, .. } | Rel::Filter { input, .. } => core(input),
            o => o,
        }
    }
    let mut offs = Vec::new();
    let mut conds = Vec::new();
    if !leaves(core(v), 0, sel, &mut offs, &mut conds) {
        return None;
    }
    // Filters above the join may also hold edge conditions.
    let mut cur = v;
    while let Rel::Project { input, .. } | Rel::Filter { input, .. } = cur {
        if let Rel::Filter { pred, .. } = cur {
            if matches!(&**input, Rel::Join { .. } | Rel::Scan { .. }) {
                conds.extend(conjuncts(pred));
            }
        }
        cur = input;
    }
    let xparent = (0..xw).find(|c| origin(x, *c).is_some_and(|o| (o.1, o.2) == (CoreRel::Node, "parent")))?;
    let is_col = |e: &Expr<S>, c: u16| matches!(e, Expr::Col { depth: 0, col } if *col == c);
    // Each non-anchor leaf: reached from an earlier leaf by a downward edge.
    for (i, &o) in offs.iter().enumerate().skip(1) {
        let reached = offs[..i].iter().any(|&a| {
            conds.iter().any(|c| match c {
                Expr::Cmp(CmpOp::Eq, l, r, _) => (is_col(l, o + xparent) && is_col(r, a + xid)) || (is_col(r, o + xparent) && is_col(l, a + xid)),
                _ => false,
            }) || (conds.iter().any(|c| matches!(c, Expr::Cmp(CmpOp::Gt, l, r, _) if is_col(l, o + xid) && is_col(r, a + xid)))
                && conds.iter().any(|c| matches!(c, Expr::Cmp(CmpOp::Lt, l, r, _) if is_col(l, o + xid) && is_col(r, a + xend))))
        });
        if !reached {
            return None;
        }
    }
    // v[sel → X], with the anchor (first leaf) semijoined with sel.
    let mut first = true;
    let mut out = v.clone();
    fn replace<S: Source>(r: &mut Rel<S>, sel: StageId, x: &Rel<S>, xid: u16, first: &mut bool) {
        match r {
            Rel::Scan { src, .. } if src.stage() == Some(sel) => {
                let scan = r.clone();
                *r = if std::mem::take(first) {
                    let member = Rel::Filter { input: Box::new(scan), pred: Expr::Cmp(CmpOp::Eq, Box::new(col(xid)), Box::new(Expr::Col { depth: 1, col: xid }), (Aff::None, Aff::None)) };
                    Rel::Filter { input: Box::new(x.clone()), pred: Expr::Exists { rel: Box::new(member), negated: false } }
                } else {
                    x.clone()
                };
            }
            Rel::Join { left, right, .. } => {
                replace(left, sel, x, xid, first);
                replace(right, sel, x, xid, first);
            }
            Rel::Project { input, .. } | Rel::Filter { input, .. } => replace(input, sel, x, xid, first),
            _ => {}
        }
    }
    replace(&mut out, sel, x, xid, &mut first);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{Rng, col as tcol, int, same as same_rows, scan as tscan, schema};

    impl Source for u32 {
        fn stage(&self) -> Option<StageId> {
            None
        }
        fn as_stage(_: StageId) -> Self {
            unreachable!("test tables have no stages")
        }
    }

    fn cmp<S>(op: CmpOp, a: Expr<S>, b: Expr<S>) -> Expr<S> {
        Expr::Cmp(op, Box::new(a), Box::new(b), (Aff::None, Aff::None))
    }

    // A file's core relations: a random tree in preorder.
    fn tree(g: &mut Rng, n: usize) -> Vec<(CoreRel, Vec<Row>)> {
        let mut parent: Vec<Option<usize>> = vec![None];
        let mut depth = vec![0usize];
        for i in 1..n {
            // Attach to a node on the current rightmost path (preorder).
            let mut path = vec![i - 1];
            while let Some(p) = parent[*path.last().unwrap()] {
                path.push(p);
            }
            let p = path[g.below(path.len() as u64) as usize];
            parent.push(Some(p));
            depth.push(depth[p] + 1);
        }
        let mut end = vec![0; n];
        for i in (0..n).rev() {
            end[i] = end[i].max(i + 1);
            if let Some(p) = parent[i] {
                end[p] = end[p].max(end[i]);
            }
        }
        let kinds = ["a", "b", "c"];
        let node: Vec<Row> = (0..n)
            .map(|i| {
                vec![
                    V::Int(i as i64),
                    V::text(kinds[g.below(3) as usize]),
                    parent[i].map_or(V::Null, |p| V::Int(p as i64)),
                    V::Int(i as i64),
                    V::Int(end[i] as i64),
                    V::Int(1),
                    V::Int(1),
                    V::Int(end[i] as i64),
                    V::Int(1),
                    V::Int(1),
                    V::Null,
                ]
            })
            .collect();
        vec![(CoreRel::File, vec![vec![V::text("f.py")]]), (CoreRel::Node, node)]
    }

    fn run(p: &Plan<FileSource>, core: &[(CoreRel, Vec<Row>)]) -> Vec<Row> {
        let scan = |s: &FileSource| -> Result<Vec<Row>, String> {
            match s {
                FileSource::Core(c) => Ok(core.iter().find(|(k, _)| k == c).map(|(_, r)| r.clone()).unwrap_or_default()),
                o => Err(format!("{o:?}")),
            }
        };
        crate::reference::evaluate_plan(p, |s| s.stage(), &scan).unwrap()
    }

    fn fcol(c: u16) -> Expr<FileSource> {
        Expr::Col { depth: 0, col: c }
    }
    fn nodes() -> Rel<FileSource> {
        Rel::Join { kind: JoinKind::Inner, left: Box::new(Rel::Scan { src: FileSource::Core(CoreRel::File), schema: CoreRel::File.schema() }), right: Box::new(Rel::Scan { src: FileSource::Core(CoreRel::Node), schema: CoreRel::Node.schema() }), on: None }
    }
    fn stage(name: &str, rel: Rel<FileSource>) -> Stage<FileSource> {
        Stage { name: name.into(), schema: rel.schema(), def: StageDef::Query { rel } }
    }
    fn plan(stages: Vec<Stage<FileSource>>, fin: Rel<FileSource>) -> Plan<FileSource> {
        Plan { clause: "t".into(), syntax: None, stages, sink: Sink::Findings { rel: fin, key: vec![], code: None, locations: vec![], violation: Violation { code: String::new(), anchor: Anchor::Span, each: None } } }
    }
    fn st(i: u32, w: usize) -> Rel<FileSource> {
        Rel::Scan { src: FileSource::Stage(i), schema: schema(w) }
    }
    fn kind_is(k: &str) -> Expr<FileSource> {
        cmp(CmpOp::Eq, fcol(2), Expr::Lit(Lit::Text(k.into())))
    }

    /// R6: a stage read once is inlined; a column whose stored affinity would convert values is
    /// not (mutant 16 inlines it and `typeof` sees the unconverted value).
    #[test]
    fn rewrite_view_inlining() {
        for seed in 0..30 {
            let mut g = Rng::new(seed);
            let n = 2 + g.below(30) as usize;
            let core = tree(&mut g, n);
            // s0: kinds of `a` nodes; read once by the sink.
            let s0 = stage("s0", Rel::Project { input: Box::new(Rel::Filter { input: Box::new(nodes()), pred: kind_is("a") }), exprs: vec![fcol(0), fcol(1), fcol(2)], schema: schema(3) });
            let fin = Rel::Project { input: Box::new(st(0, 3)), exprs: vec![fcol(0), fcol(1)], schema: schema(2) };
            let p = plan(vec![s0], fin);
            let (q, done) = inline_views(p.clone());
            assert_eq!(done, vec!["s0".to_string()]);
            assert!(q.stages.is_empty());
            assert!(same_rows(&run(&p, &core), &run(&q, &core)), "seed {seed}");
            // A TEXT column fed an integer literal in one arm: storing converts it.
            let arm = |e: Expr<FileSource>| Rel::Project { input: Box::new(Rel::Unit), exprs: vec![e], schema: vec![Column::new("v", Aff::Text)] };
            let u = Rel::Union { all: true, left: Box::new(arm(Expr::Lit(Lit::Text("x".into())))), right: Box::new(arm(Expr::Lit(Lit::Int(5)))) };
            let s1 = Stage { name: "s1".into(), schema: vec![Column::new("v", Aff::Text)], def: StageDef::Query { rel: u } };
            let fin = Rel::Project { input: Box::new(Rel::Scan { src: FileSource::Stage(0), schema: vec![Column::new("v", Aff::Text)] }), exprs: vec![Expr::Func(Func::Typeof, vec![fcol(0)])], schema: schema(1) };
            let p = plan(vec![s1], fin);
            let (q, _) = inline_views(p.clone());
            assert!(same_rows(&run(&p, &core), &run(&q, &core)), "affinity, seed {seed}");
        }
    }

    /// R7: UNION ALL of selections of one relation equals its fused form; arms over different
    /// relations are not fused (mutant 17 fuses them, over the first arm's base).
    #[test]
    fn rewrite_union_of_selections() {
        for seed in 0..200 {
            let mut g = Rng::new(seed);
            let tables = vec![g.rows(3, 8), g.rows(3, 8)];
            let arm = |t: u32, p: Expr<u32>, e: Vec<Expr<u32>>| Rel::Project { input: Box::new(Rel::Filter { input: Box::new(tscan(t, 3)), pred: p }), exprs: e, schema: schema(2) };
            let u = Rel::Union {
                all: true,
                left: Box::new(Rel::Union { all: true, left: Box::new(arm(0, cmp(CmpOp::Eq, tcol(0), int(1)), vec![tcol(1), tcol(2)])), right: Box::new(arm(0, cmp(CmpOp::Gt, tcol(1), int(0)), vec![tcol(0), tcol(0)])) }),
                right: Box::new(arm(0, Expr::IsNull { e: Box::new(tcol(2)), negated: false }, vec![tcol(2), int(7)])),
            };
            let f = fuse_union(&u).expect("one base");
            assert_eq!(f.arms.len(), 3);
            assert!(same_rows(&crate::testkit::eval(&u, &tables).unwrap(), &crate::testkit::eval(&f.to_rel(), &tables).unwrap()), "seed {seed}");
            let mixed = Rel::Union { all: true, left: Box::new(arm(0, cmp(CmpOp::Eq, tcol(0), int(1)), vec![tcol(1), tcol(2)])), right: Box::new(arm(1, cmp(CmpOp::Eq, tcol(0), int(1)), vec![tcol(1), tcol(2)])) };
            if let Some(f) = fuse_union(&mixed) {
                assert!(same_rows(&crate::testkit::eval(&mixed, &tables).unwrap(), &crate::testkit::eval(&f.to_rel(), &tables).unwrap()), "mixed, seed {seed}");
            }
        }
    }

    /// R8: unread stage columns are dropped; DISTINCT's prefix is a read (mutant 18 prunes it).
    #[test]
    fn rewrite_projection_pruning() {
        for seed in 0..60 {
            let mut g = Rng::new(seed);
            let n = 2 + g.below(30) as usize;
            let core = tree(&mut g, n);
            // s0: (path, id, kind, parent); s1: DISTINCT (kind, parent) of s0 projected with id;
            // the sink reads s1's kind only.
            let s0 = stage("s0", Rel::Project { input: Box::new(nodes()), exprs: vec![fcol(0), fcol(1), fcol(2), fcol(3)], schema: schema(4) });
            let s1 = stage("s1", Rel::Distinct { input: Box::new(Rel::Project { input: Box::new(st(0, 4)), exprs: vec![fcol(2), fcol(3), fcol(1)], schema: schema(3) }), prefix: 2 });
            let fin = Rel::Project { input: Box::new(st(1, 3)), exprs: vec![fcol(0)], schema: schema(1) };
            let p = plan(vec![s0, s1], fin);
            let (q, pruned) = prune_columns(p.clone());
            // s1 stores only what the sink reads; its DISTINCT still reads (kind, parent) of s0.
            assert_eq!(pruned, vec![("s0".to_string(), 2, 4), ("s1".to_string(), 1, 3)]);
            assert!(same_rows(&run(&p, &core), &run(&q, &core)), "seed {seed}");
        }
    }

    fn descendants() -> Rel<FileSource> {
        // walk(p, root, node) over the a/b roots, E = node rows (path, parent, id).
        let roots = Rel::Filter { input: Box::new(nodes()), pred: Expr::Not(Box::new(kind_is("c"))) };
        let init = Rel::Project { input: Box::new(roots), exprs: vec![fcol(0), fcol(1), fcol(1)], schema: schema(3) };
        let edges = Rel::Project { input: Box::new(Rel::Filter { input: Box::new(nodes()), pred: Expr::IsNull { e: Box::new(fcol(3)), negated: true } }), exprs: vec![fcol(0), fcol(3), fcol(1)], schema: schema(3) };
        let on = Expr::And(Box::new(cmp(CmpOp::Eq, fcol(3), fcol(0))), Box::new(cmp(CmpOp::Eq, fcol(4), fcol(2))));
        let step = Rel::Project { input: Box::new(Rel::Join { kind: JoinKind::Inner, left: Box::new(Rel::Local { id: 0, schema: schema(3) }), right: Box::new(edges), on: Some(on) }), exprs: vec![fcol(0), fcol(1), fcol(5)], schema: schema(3) };
        Rel::With { bindings: vec![(0, Binding::Fix { init: vec![init], step: vec![step], set: true, schema: schema(3) })], body: Box::new(Rel::Local { id: 0, schema: schema(3) }) }
    }

    /// R9: reachability over the tree edge becomes the interval join where the stage's order is
    /// unobserved; under `row_number()` it is kept (mutant 19 rewrites it and the numbers move).
    #[test]
    fn rewrite_tree_reachability_to_interval() {
        for seed in 0..60 {
            let mut g = Rng::new(seed);
            let n = 2 + g.below(40) as usize;
            let core = tree(&mut g, n);
            // Order-insensitive reader: DISTINCT (root, kind of node).
            let reader = Rel::Distinct { input: Box::new(Rel::Project { input: Box::new(Rel::Join { kind: JoinKind::Inner, left: Box::new(st(0, 3)), right: Box::new(nodes()), on: Some(cmp(CmpOp::Eq, fcol(4), fcol(2))) }), exprs: vec![fcol(1), fcol(5)], schema: schema(2) }), prefix: 2 };
            let p = plan(vec![stage("descendants", descendants())], reader);
            let (q, done) = reach_to_interval(p.clone());
            assert_eq!(done.len(), 1);
            let (a, b) = (run(&p, &core), run(&q, &core));
            assert_eq!(crate::reference::multiset(&a), crate::reference::multiset(&b), "seed {seed}");
            // Order-sensitive reader: row_number() over the stage in its order.
            let rn = Rel::Window { input: Box::new(st(0, 3)), funcs: vec![WindowFn { kind: WindowKind::RowNumber, partition: vec![], order: vec![], total: Total::InputOrder }] };
            let p = plan(vec![stage("descendants", descendants())], rn);
            let (q, _) = reach_to_interval(p.clone());
            assert_eq!(crate::reference::multiset(&run(&p, &core)), crate::reference::multiset(&run(&q, &core)), "ordered, seed {seed}");
        }
    }

    /// R10: the probe's candidates, re-checked, are the join's right rows in table order, each
    /// once (mutant 20 keeps a row matched by two disjuncts twice); a disjunct without a key is
    /// not probed.
    #[test]
    fn rewrite_disjunctive_lookups() {
        let ons: Vec<Expr<u32>> = vec![
            Expr::Or(Box::new(cmp(CmpOp::Eq, tcol(0), tcol(3))), Box::new(cmp(CmpOp::Eq, tcol(1), tcol(4)))),
            Expr::And(Box::new(Expr::Or(Box::new(Expr::And(Box::new(cmp(CmpOp::Eq, tcol(0), tcol(3))), Box::new(cmp(CmpOp::Gt, tcol(2), int(0))))), Box::new(cmp(CmpOp::Eq, tcol(5), tcol(1))))), Box::new(Expr::IsNull { e: Box::new(tcol(4)), negated: true })),
            Expr::Or(Box::new(cmp(CmpOp::Eq, tcol(0), tcol(3))), Box::new(cmp(CmpOp::Lt, tcol(1), tcol(4)))),
        ];
        for (k, on) in ons.iter().enumerate() {
            let probe = or_probe(on, 3);
            assert_eq!(probe.is_some(), k < 2, "case {k}: a disjunct without a key is not probed");
            let Some(probe) = probe else { continue };
            for seed in 0..200 {
                let mut g = Rng::new(seed * 7 + k as u64);
                let tables = vec![g.rows(3, 6), g.rows(3, 6)];
                let join = Rel::Join { kind: JoinKind::Inner, left: Box::new(tscan(0, 3)), right: Box::new(tscan(1, 3)), on: Some(on.clone()) };
                let want = crate::testkit::eval(&join, &tables).unwrap();
                let mut got = Vec::new();
                for l in &tables[0] {
                    for j in probe.matches(l, &tables[1]).unwrap() {
                        let mut row = l.clone();
                        row.extend(tables[1][j].iter().cloned());
                        got.push(row);
                    }
                }
                assert!(same_rows(&want, &got), "case {k} seed {seed}");
            }
        }
    }

    /// R11: a stage over a subtree-closed selection equals the all-nodes stage semijoined at its
    /// anchor; a selection that is not closed is not rewritten (mutant 21 rewrites it).
    #[test]
    fn rewrite_selection_sharing() {
        // Closed: kind 'a', or a proper ancestor of kind 'a' (any[X, inside{X, stopBy: end}]).
        let anc = Rel::Filter {
            input: Box::new(nodes()),
            pred: Expr::And(Box::new(Expr::And(Box::new(cmp(CmpOp::Lt, fcol(1), Expr::Col { depth: 1, col: 1 })), Box::new(cmp(CmpOp::Gt, fcol(8), Expr::Col { depth: 1, col: 1 })))), Box::new(kind_is("a"))),
        };
        let closed = Expr::Or(Box::new(kind_is("a")), Box::new(Expr::Exists { rel: Box::new(anc), negated: false }));
        let open = kind_is("a");
        for (k, pred) in [closed, open].into_iter().enumerate() {
            let sel = Rel::Filter { input: Box::new(nodes()), pred };
            // v: (f.id, c.id, c.kind) over sel f ⋈ sel c ON c.parent = f.id.
            let v = Rel::Project { input: Box::new(Rel::Join { kind: JoinKind::Inner, left: Box::new(st(0, 12)), right: Box::new(st(0, 12)), on: Some(cmp(CmpOp::Eq, fcol(15), fcol(1))) }), exprs: vec![fcol(1), fcol(13), fcol(14)], schema: schema(3) };
            let shared = share_selection(&v, 0, &sel);
            assert_eq!(shared.is_some(), k == 0 || crate::mutant::on(21), "case {k}");
            let Some(shared) = shared else { continue };
            for seed in 0..60 {
                let mut g = Rng::new(seed + 1000 * k as u64);
                let n = 2 + g.below(40) as usize;
            let core = tree(&mut g, n);
                let p = plan(vec![stage("sel", sel.clone())], v.clone());
                let q = plan(vec![stage("sel", sel.clone())], shared.clone());
                assert!(same_rows(&run(&p, &core), &run(&q, &core)), "case {k} seed {seed}");
            }
        }
    }
}
