//! Classification (IR.md § 7, INVARIANTS I6): a plan is per-content iff every leaf it reads is a
//! per-content relation. The declaration (`partitionBy`, or a clause form's implicit partition)
//! is checked against the inferred class, never trusted.
//!
//! - A per-file plan is mapped `Rel<AnySource>` → `Rel<FileSource>` by [`file_source`]; any
//!   cross-file leaf is a compile error (the leak test: `cargo test -p rerail-facts leak`).
//! - A repository clause is split by file-locality ([`Loc`]): every maximal file-local subtree
//!   without free outer references becomes an [`ExportedRelation`] computed per content
//!   (projected to the columns the repository side reads, distinct when it reads them as a set);
//!   the rest is a `Plan<RepoSource>` that scans only exported relations and its own stages.
//! - Repository plans are audited for order: `row_number()`, `LIMIT`, `group_concat`,
//!   `json_group_array` and aggregate bare columns need an order the dataflow can maintain, i.e.
//!   a total order proven from candidate keys; otherwise they are refused (PHASE-2 kill
//!   criterion).
use crate::front::rule::{ClauseBody, Front};
use crate::front::sql::visit_expr;
use crate::ir::{
    AggFn, AnySource, Binding, CmpOp, ExportId, ExportedRelation, Expr, FileSource, Func, LocalId, Partition, Plan, Projection, Refusal, Rel, RepoSource, Sink, Stage, StageDef, Total, WindowKind, map_rel,
};
use crate::sql::value::Aff;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// ---------------------------------------------------------------------------------------------
// Per-file mapping

/// The only way a source enters a per-file plan: every cross-file leaf is refused here.
pub fn file_source(src: AnySource, input_cross: &[Option<String>]) -> Result<FileSource, String> {
    match src {
        AnySource::Input(i) => match &input_cross[i as usize] {
            // Mutant 7: a per-file plan accepts a cross-file input.
            Some(_) if crate::mutant::on(7) => Ok(FileSource::Input(i)),
            Some(c) => Err(format!("cross-file read in a per-file plan: {c}")),
            None => Ok(FileSource::Input(i)),
        },
        AnySource::Syntax(t) => Ok(FileSource::Syntax(t)),
        AnySource::Stage(s) => Ok(FileSource::Stage(s)),
        AnySource::ReferenceEdges if crate::mutant::on(7) => Ok(FileSource::Stage(u32::MAX)),
        AnySource::ReferenceEdges => Err("cross-file read in a per-file plan: `reference_edges` (the reference graph)".into()),
    }
}

fn to_file(r: Rel<AnySource>, cross: &[Option<String>], stage_map: &dyn Fn(u32) -> u32) -> Result<Rel<FileSource>, String> {
    map_rel(r, &mut |s| match file_source(s, cross)? {
        FileSource::Stage(i) => Ok(FileSource::Stage(stage_map(i))),
        o => Ok(o),
    })
}

fn stage_to_file(st: Stage<AnySource>, cross: &[Option<String>], stage_map: &dyn Fn(u32) -> u32) -> Result<Stage<FileSource>, String> {
    let def = match st.def {
        StageDef::Query { rel } => StageDef::Query { rel: to_file(rel, cross, stage_map)? },
        StageDef::Reach { roots, edges, graph } => StageDef::Reach { roots: to_file(roots, cross, stage_map)?, edges: edges.map(|e| to_file(e, cross, stage_map)).transpose()?, graph },
    };
    Ok(Stage { name: st.name, schema: st.schema, def })
}

// ---------------------------------------------------------------------------------------------
// File-locality

/// How a relation of a repository partition depends on the contents.
#[derive(Clone, Debug, PartialEq)]
pub enum Loc {
    /// Independent of every content (literals, `json_each` over literals).
    Const,
    /// The union over contents of the same relation computed from one content; the listed
    /// columns hold that content's path (none: the path is not carried).
    Local(Vec<u16>),
    /// Inside a correlated subquery: computed from the enclosing row's content (and
    /// constants) only; the listed columns hold its path.
    Bound(Vec<u16>),
    Global,
}

struct Env<'a> {
    cross: &'a [Option<String>],
    stages: Vec<Loc>,
    locals: HashMap<LocalId, Loc>,
}

fn width<S>(r: &Rel<S>) -> usize {
    r.schema().len()
}

/// Visits every subquery relation directly inside an expression (not nested deeper).
fn subqueries<S>(e: &Expr<S>, f: &mut dyn FnMut(&Rel<S>)) {
    visit_expr(e, &mut |x| match x {
        Expr::Exists { rel, .. } | Expr::InRel { rel, .. } | Expr::Scalar(rel) => f(rel),
        _ => {}
    });
}

fn conjuncts<'e, S>(e: &'e Expr<S>, out: &mut Vec<&'e Expr<S>>) {
    match e {
        Expr::And(a, b) => {
            conjuncts(a, out);
            conjuncts(b, out);
        }
        o => out.push(o),
    }
}

struct Uf(Vec<usize>);
impl Uf {
    fn find(&mut self, x: usize) -> usize {
        if self.0[x] != x {
            let r = self.find(self.0[x]);
            self.0[x] = r;
        }
        self.0[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.0[a] = b;
    }
}

enum Part<'r, S> {
    Leaf(usize, &'r Rel<S>),
    /// A `json_each` element at this offset: rows depend on the row before it.
    Json,
}

fn flatten<'r, S>(r: &'r Rel<S>, off: usize, parts: &mut Vec<Part<'r, S>>, conj: &mut Vec<(usize, &'r Expr<S>)>, args: &mut Vec<(usize, &'r Expr<S>)>) {
    match r {
        Rel::Filter { input, pred } => {
            flatten(input, off, parts, conj, args);
            let mut cs = Vec::new();
            conjuncts(pred, &mut cs);
            conj.extend(cs.into_iter().map(|c| (off, c)));
        }
        Rel::Join { left, right, on, .. } => {
            flatten(left, off, parts, conj, args);
            flatten(right, off + width(left), parts, conj, args);
            if let Some(on) = on {
                let mut cs = Vec::new();
                conjuncts(on, &mut cs);
                conj.extend(cs.into_iter().map(|c| (off, c)));
            }
        }
        Rel::JsonEach { input, arg, on, .. } => {
            flatten(input, off, parts, conj, args);
            parts.push(Part::Json);
            args.push((off, arg));
            if let Some(on) = on {
                let mut cs = Vec::new();
                conjuncts(on, &mut cs);
                conj.extend(cs.into_iter().map(|c| (off, c)));
            }
        }
        leaf => parts.push(Part::Leaf(off, leaf)),
    }
}

fn is_block<S>(r: &Rel<S>) -> bool {
    matches!(r, Rel::Filter { .. } | Rel::Join { .. } | Rel::JsonEach { .. })
}

impl Env<'_> {
    /// Whether every subquery in `exprs` stays within the row's content (`path`: the row's
    /// path columns; `None` when the row is not per content).
    fn subs_safe<'e>(&mut self, exprs: impl IntoIterator<Item = &'e Expr<AnySource>>, path: Option<&[u16]>) -> bool {
        let mut ok = true;
        for e in exprs {
            subqueries(e, &mut |rel| {
                if ok {
                    ok = matches!(self.loc(rel, path), Loc::Const | Loc::Bound(_));
                }
            });
        }
        ok
    }

    /// `outer`: the path columns of the enclosing row (depth 1), when it is per content.
    pub fn loc(&mut self, r: &Rel<AnySource>, outer: Option<&[u16]>) -> Loc {
        if is_block(r) {
            return self.block(r, outer);
        }
        match r {
            Rel::Scan { src, .. } => match src {
                AnySource::Input(i) if self.cross[*i as usize].is_some() => Loc::Global,
                AnySource::Input(_) | AnySource::Syntax(_) => Loc::Local(vec![0]),
                AnySource::Stage(s) => self.stages[*s as usize].clone(),
                AnySource::ReferenceEdges => Loc::Global,
            },
            Rel::Local { id, .. } => self.locals.get(id).cloned().unwrap_or(Loc::Global),
            Rel::Unit => Loc::Const,
            Rel::Project { input, exprs, .. } => {
                let li = self.loc(input, outer);
                let path = path_of(&li);
                if !self.subs_safe(exprs, path.as_deref()) && li != Loc::Global {
                    return Loc::Global;
                }
                let map = |p: &[u16]| exprs.iter().enumerate().filter(|(_, e)| matches!(e, Expr::Col { depth: 0, col } if p.contains(col))).map(|(j, _)| j as u16).collect::<Vec<_>>();
                match li {
                    Loc::Local(p) => Loc::Local(map(&p)),
                    Loc::Bound(p) => Loc::Bound(map(&p)),
                    o => o,
                }
            }
            Rel::Aggregate { input, keys, aggs } => {
                let li = self.loc(input, outer);
                let path = path_of(&li);
                let mut exprs: Vec<&Expr<AnySource>> = keys.iter().collect();
                exprs.extend(aggs.iter().flat_map(|a| a.arg.iter().chain(a.sep.iter())));
                if !self.subs_safe(exprs, path.as_deref()) {
                    return Loc::Global;
                }
                match li {
                    Loc::Local(p) if keys.iter().any(|k| matches!(k, Expr::Col { depth: 0, col } if p.contains(col))) => Loc::Local(p),
                    Loc::Local(_) => Loc::Global,
                    o => o,
                }
            }
            Rel::Window { input, funcs } => {
                let li = self.loc(input, outer);
                let path = path_of(&li);
                let mut exprs: Vec<&Expr<AnySource>> = Vec::new();
                for f in funcs {
                    exprs.extend(f.partition.iter());
                    exprs.extend(f.order.iter().map(|o| &o.0));
                    if let WindowKind::Min(e) | WindowKind::Max(e) = &f.kind {
                        exprs.push(e);
                    }
                }
                if !self.subs_safe(exprs, path.as_deref()) {
                    return Loc::Global;
                }
                match li {
                    Loc::Local(p) if funcs.iter().all(|f| f.partition.iter().any(|k| matches!(k, Expr::Col { depth: 0, col } if p.contains(col)))) => Loc::Local(p),
                    Loc::Local(_) => Loc::Global,
                    o => o,
                }
            }
            Rel::Distinct { input, prefix } => match self.loc(input, outer) {
                Loc::Local(p) => {
                    let q: Vec<u16> = p.into_iter().filter(|c| c < prefix).collect();
                    if q.is_empty() { Loc::Global } else { Loc::Local(q) }
                }
                Loc::Bound(p) => Loc::Bound(p.into_iter().filter(|c| c < prefix).collect()),
                o => o,
            },
            Rel::Union { all, left, right } => {
                let (l, rr) = (self.loc(left, outer), self.loc(right, outer));
                union_loc(l, rr, *all)
            }
            Rel::Sort { input, limit, offset, .. } => {
                let li = self.loc(input, outer);
                if !self.subs_safe(limit.iter().chain(offset.iter()), None) {
                    return Loc::Global;
                }
                match li {
                    Loc::Local(_) if limit.is_some() || offset.is_some() => Loc::Global,
                    o => o,
                }
            }
            Rel::With { bindings, body } => {
                for (id, b) in bindings {
                    let l = self.binding(*id, b, outer);
                    self.locals.insert(*id, l);
                }
                self.loc(body, outer)
            }
            Rel::Filter { .. } | Rel::Join { .. } | Rel::JsonEach { .. } => unreachable!(),
        }
    }

    fn binding(&mut self, id: LocalId, b: &Binding<AnySource>, outer: Option<&[u16]>) -> Loc {
        match b {
            Binding::Plain(r) => self.loc(r, outer),
            Binding::Fix { init, step, set, .. } => {
                let mut cur = init.iter().map(|r| self.loc(r, outer)).reduce(|a, b| union_loc(a, b, true)).unwrap_or(Loc::Const);
                // Two rounds reach the fixpoint of this finite lattice walk.
                for _ in 0..3 {
                    self.locals.insert(id, cur.clone());
                    let s = step.iter().map(|r| self.loc(r, outer)).reduce(|a, b| union_loc(a, b, true)).unwrap_or(Loc::Const);
                    let next = union_loc(cur.clone(), s, !*set);
                    if next == cur {
                        break;
                    }
                    cur = next;
                }
                cur
            }
        }
    }

    /// A Filter/Join/JsonEach block: its leaves must be connected by path equalities.
    fn block(&mut self, r: &Rel<AnySource>, outer: Option<&[u16]>) -> Loc {
        let mut parts = Vec::new();
        let mut conj = Vec::new();
        let mut args = Vec::new();
        flatten(r, 0, &mut parts, &mut conj, &mut args);
        let w = width(r);
        let mut uf = Uf((0..w + 1).collect());
        let outer_node = w; // stands for the enclosing row's path
        for (off, c) in &conj {
            if let Expr::Cmp(CmpOp::Eq, a, b, _) = c {
                match (&**a, &**b) {
                    (Expr::Col { depth: 0, col: x }, Expr::Col { depth: 0, col: y }) => uf.union(off + *x as usize, off + *y as usize),
                    (Expr::Col { depth: 0, col: x }, Expr::Col { depth: 1, col: y }) | (Expr::Col { depth: 1, col: y }, Expr::Col { depth: 0, col: x }) => {
                        if outer.is_some_and(|o| o.contains(y)) {
                            uf.union(off + *x as usize, outer_node);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut locals: Vec<Vec<usize>> = Vec::new();
        let mut bound = false;
        for p in &parts {
            let Part::Leaf(off, leaf) = p else { continue };
            match self.loc(leaf, outer) {
                Loc::Global => return Loc::Global,
                Loc::Const => {}
                Loc::Bound(pc) => {
                    bound = true;
                    for c in pc {
                        uf.union(off + c as usize, outer_node);
                    }
                }
                Loc::Local(pc) => {
                    if pc.is_empty() {
                        // Per content, path not carried: only alone.
                        locals.push(vec![usize::MAX]);
                    } else {
                        locals.push(pc.iter().map(|c| off + *c as usize).collect());
                    }
                }
            }
        }
        let result = if locals.is_empty() {
            if bound { Loc::Bound(vec![]) } else { Loc::Const }
        } else if locals.len() == 1 && locals[0] == [usize::MAX] && !bound {
            Loc::Local(vec![])
        } else if locals.iter().any(|l| l == &[usize::MAX]) {
            Loc::Global
        } else {
            let root = uf.find(locals[0][0]);
            // Mutant 15 (dropped side condition): a join of per-content relations without a path
            // equality is taken as per content.
            if !crate::mutant::on(15) && !locals.iter().all(|l| uf.find(l[0]) == root) {
                Loc::Global
            } else {
                let cols: Vec<u16> = (0..w).filter(|c| uf.find(*c) == root).map(|c| c as u16).collect();
                if uf.find(outer_node) == root {
                    Loc::Bound(cols)
                } else if bound {
                    Loc::Global
                } else {
                    Loc::Local(cols)
                }
            }
        };
        let path = path_of(&result);
        if result != Loc::Global {
            let exprs: Vec<&Expr<AnySource>> = conj.iter().map(|c| c.1).chain(args.iter().map(|a| a.1)).collect();
            if !self.subs_safe(exprs, path.as_deref()) {
                return Loc::Global;
            }
        }
        result
    }
}

fn path_of(l: &Loc) -> Option<Vec<u16>> {
    match l {
        Loc::Local(p) | Loc::Bound(p) => Some(p.clone()),
        Loc::Const => Some(vec![]),
        Loc::Global => None,
    }
}

fn union_loc(l: Loc, r: Loc, all: bool) -> Loc {
    let inter = |p: Vec<u16>, q: Vec<u16>| p.into_iter().filter(|c| q.contains(c)).collect::<Vec<_>>();
    match (l, r) {
        (Loc::Const, Loc::Const) => Loc::Const,
        (Loc::Bound(p), Loc::Bound(q)) => Loc::Bound(inter(p, q)),
        (Loc::Bound(p), Loc::Const) | (Loc::Const, Loc::Bound(p)) => Loc::Bound(if all { vec![] } else { p.into_iter().take(0).collect() }),
        (Loc::Local(p), Loc::Local(q)) => {
            let i = inter(p, q);
            if all || !i.is_empty() { Loc::Local(i) } else { Loc::Global }
        }
        _ => Loc::Global,
    }
}

// ---------------------------------------------------------------------------------------------
// Column demand and free outer references

/// Columns of the row at nesting `level` (0: the operator's own row) read by an expression,
/// including from inside its subqueries.
fn expr_cols<S>(e: &Expr<S>, level: u16, out: &mut BTreeSet<u16>) {
    visit_expr(e, &mut |x| match x {
        Expr::Col { depth, col } if *depth == level => {
            out.insert(*col);
        }
        Expr::Exists { rel, .. } | Expr::InRel { rel, .. } | Expr::Scalar(rel) => rel_cols(rel, level + 1, out),
        _ => {}
    });
}

/// Columns of the row `level` levels out read anywhere inside a relation.
fn rel_cols<S>(r: &Rel<S>, level: u16, out: &mut BTreeSet<u16>) {
    each_expr(r, &mut |e| expr_cols(e, level, out));
    each_child(r, &mut |c| rel_cols(c, level, out));
}

fn each_expr<S>(r: &Rel<S>, f: &mut dyn FnMut(&Expr<S>)) {
    match r {
        Rel::JsonEach { arg, on, .. } => {
            f(arg);
            on.iter().for_each(|e| f(e));
        }
        Rel::Join { on, .. } => on.iter().for_each(|e| f(e)),
        Rel::Filter { pred, .. } => f(pred),
        Rel::Project { exprs, .. } => exprs.iter().for_each(|e| f(e)),
        Rel::Aggregate { keys, aggs, .. } => {
            keys.iter().for_each(|e| f(e));
            aggs.iter().for_each(|a| a.arg.iter().chain(a.sep.iter()).for_each(|e| f(e)));
        }
        Rel::Window { funcs, .. } => {
            for w in funcs {
                w.partition.iter().for_each(|e| f(e));
                w.order.iter().for_each(|o| f(&o.0));
                if let WindowKind::Min(e) | WindowKind::Max(e) = &w.kind {
                    f(e);
                }
            }
        }
        Rel::Sort { limit, offset, .. } => limit.iter().chain(offset.iter()).for_each(|e| f(e)),
        _ => {}
    }
}

fn each_child<S>(r: &Rel<S>, f: &mut dyn FnMut(&Rel<S>)) {
    match r {
        Rel::JsonEach { input, .. } | Rel::Filter { input, .. } | Rel::Project { input, .. } | Rel::Aggregate { input, .. } | Rel::Window { input, .. } | Rel::Distinct { input, .. } | Rel::Sort { input, .. } => f(input),
        Rel::Join { left, right, .. } | Rel::Union { left, right, .. } => {
            f(left);
            f(right);
        }
        Rel::With { bindings, body } => {
            for (_, b) in bindings {
                match b {
                    Binding::Plain(r) => f(r),
                    Binding::Fix { init, step, .. } => init.iter().chain(step.iter()).for_each(|r| f(r)),
                }
            }
            f(body);
        }
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => {}
    }
}

/// Whether a relation reads a row outside itself (a correlation into an enclosing query).
fn has_free_outer<S>(r: &Rel<S>) -> bool {
    let mut cols = BTreeSet::new();
    rel_cols(r, 1, &mut cols);
    // Deeper escapes: depth > nesting is also free; `rel_cols` at level 1 catches depth 1 at
    // the top. Check depths 2.. conservatively.
    let mut deeper = false;
    fn deep<S>(r: &Rel<S>, level: u16, found: &mut bool) {
        each_expr(r, &mut |e| {
            visit_expr(e, &mut |x| match x {
                Expr::Col { depth, .. } if *depth > level => *found = true,
                Expr::Exists { rel, .. } | Expr::InRel { rel, .. } | Expr::Scalar(rel) => deep(rel, level + 1, found),
                _ => {}
            })
        });
        each_child(r, &mut |c| deep(c, level, found));
    }
    deep(r, 0, &mut deeper);
    !cols.is_empty() || deeper
}

fn reads_locals<S>(r: &Rel<S>, out: &mut BTreeSet<LocalId>) {
    if let Rel::Local { id, .. } = r {
        out.insert(*id);
    }
    each_expr(r, &mut |e| {
        subqueries(e, &mut |s| reads_locals(s, out));
    });
    each_child(r, &mut |c| reads_locals(c, out));
}

fn binds_locals<S>(r: &Rel<S>, out: &mut BTreeSet<LocalId>) {
    if let Rel::With { bindings, .. } = r {
        out.extend(bindings.iter().map(|b| b.0));
    }
    each_expr(r, &mut |e| subqueries(e, &mut |s| binds_locals(s, out)));
    each_child(r, &mut |c| binds_locals(c, out));
}

/// A subtree is self-contained when every `Local` it reads is bound inside it.
fn self_contained<S>(r: &Rel<S>) -> bool {
    let (mut reads, mut binds) = (BTreeSet::new(), BTreeSet::new());
    reads_locals(r, &mut reads);
    binds_locals(r, &mut binds);
    reads.is_subset(&binds)
}

// ---------------------------------------------------------------------------------------------
// Candidate keys and the order audit

type Keys = Vec<BTreeSet<u16>>;

fn col_of<S>(e: &Expr<S>) -> Option<u16> {
    match e {
        Expr::Col { depth: 0, col } => Some(*col),
        _ => None,
    }
}

/// Candidate keys of a source relation.
trait KeySource {
    fn keys(&self, schema: &[crate::ir::Column], stages: &[Keys], exports: &[Keys]) -> Keys;
}

impl KeySource for RepoSource {
    fn keys(&self, _: &[crate::ir::Column], stages: &[Keys], exports: &[Keys]) -> Keys {
        match self {
            RepoSource::Exported(e) => exports[*e as usize].clone(),
            RepoSource::Stage(s) => stages[*s as usize].clone(),
        }
    }
}

impl KeySource for FileSource {
    /// Declared structure of one content's syntax tables (spec-rel § 2.3–2.6): one row per kept
    /// node keyed by `id` (nodes, ownership), one per edge keyed by `dst`. Inputs are bags.
    fn keys(&self, schema: &[crate::ir::Column], stages: &[Keys], _: &[Keys]) -> Keys {
        let col = |n: &str| schema.iter().position(|c| c.name == n).map(|i| i as u16);
        match self {
            FileSource::Syntax(crate::ir::SyntaxTable::Nodes | crate::ir::SyntaxTable::Ownership) => col("id").map(|c| vec![BTreeSet::from([c])]).unwrap_or_default(),
            FileSource::Syntax(crate::ir::SyntaxTable::Edges) => col("dst").map(|c| vec![BTreeSet::from([c])]).unwrap_or_default(),
            FileSource::Stage(s) => stages[*s as usize].clone(),
            FileSource::Input(_) => vec![],
            // SCHEMA § 1.2.1: the declared keys of the core relations.
            FileSource::Core(c) => {
                let key: &[&str] = match c {
                    crate::ir::CoreRel::Node | crate::ir::CoreRel::Identifier | crate::ir::CoreRel::StringValue | crate::ir::CoreRel::StringValueUnknown => &["id"],
                    crate::ir::CoreRel::Child | crate::ir::CoreRel::Field => &["child"],
                    crate::ir::CoreRel::Line => &["no"],
                    crate::ir::CoreRel::File | crate::ir::CoreRel::SyntaxError => &[],
                };
                key.iter().filter_map(|k| col(k)).map(|c| BTreeSet::from([c])).collect()
            }
        }
    }
}

struct KeyEnv {
    stages: Vec<Keys>,
    locals: HashMap<LocalId, Keys>,
    exports: Vec<Keys>,
}

impl KeyEnv {
    fn keys<S: KeySource + Clone>(&mut self, r: &Rel<S>) -> Keys {
        let all = |w: usize| vec![(0..w as u16).collect::<BTreeSet<u16>>()];
        match r {
            Rel::Scan { src, schema } => src.keys(schema, &self.stages, &self.exports),
            Rel::Local { id, .. } => self.locals.get(id).cloned().unwrap_or_default(),
            Rel::Unit => all(0),
            Rel::Sort { input, .. } => self.keys(input),
            // K3/K4: a column pinned by an equality to a constant or to the enclosing row (no
            // affinity conversion, so every kept row's value is equal to the others' under the
            // key notion, DKey equality) drops out of every key: `rn = 1` over `row_number()`
            // partitioned by p leaves p a key.
            Rel::Filter { input, pred } => {
                let pinned = pinned_cols(pred);
                let mut ks = self.keys(input);
                if !pinned.is_empty() {
                    let more: Keys = ks.iter().map(|k| k.difference(&pinned).copied().collect()).collect();
                    ks.extend(more);
                }
                ks
            }
            Rel::Project { input, exprs, .. } => {
                // A key survives at every choice of output position for each of its columns (a
                // column projected twice, e.g. as an output and as an ORDER BY key).
                let mut out: Keys = Vec::new();
                for k in self.keys(input) {
                    let mut variants: Vec<BTreeSet<u16>> = vec![BTreeSet::new()];
                    for c in &k {
                        let pos: Vec<u16> = exprs.iter().enumerate().filter(|(_, e)| col_of(e) == Some(*c)).map(|(j, _)| j as u16).collect();
                        variants = variants.iter().flat_map(|v| pos.iter().map(move |p| v.iter().copied().chain([*p]).collect())).take(16).collect();
                    }
                    out.extend(variants);
                }
                out
            }
            Rel::Distinct { input, prefix } => {
                let mut k: Keys = self.keys(input).into_iter().filter(|k| k.iter().all(|c| c < prefix)).collect();
                k.push((0..*prefix).collect());
                k
            }
            Rel::Union { all: false, left, .. } => all(width(left)),
            Rel::Union { all: true, .. } | Rel::JsonEach { .. } => vec![],
            Rel::Aggregate { input, keys, .. } => {
                let _ = input;
                match keys.iter().map(col_of).collect::<Option<BTreeSet<u16>>>() {
                    Some(k) => vec![k],
                    None => vec![],
                }
            }
            Rel::Window { input, funcs } => {
                let w = width(input) as u16;
                let mut k = self.keys(input);
                for (i, f) in funcs.iter().enumerate() {
                    if matches!(f.kind, WindowKind::RowNumber) {
                        if let Some(mut p) = f.partition.iter().map(col_of).collect::<Option<BTreeSet<u16>>>() {
                            p.insert(w + i as u16);
                            k.push(p);
                        }
                    }
                }
                k
            }
            Rel::Join { left, right, kind, on } => {
                let wl = width(left) as u16;
                let (kl, kr) = (self.keys(left), self.keys(right));
                let mut out = Vec::new();
                for a in &kl {
                    for b in &kr {
                        out.push(a.iter().copied().chain(b.iter().map(|c| c + wl)).collect());
                    }
                }
                // K2: a side whose key is equated, column for column without affinity
                // conversion, to the other side's columns matches at most one row per row of the
                // other side, which keeps the other side's keys (a left join only on the right:
                // null-extended rows share the right columns).
                let (lk, rk, _) = crate::ir::equi_keys(on.as_ref(), wl as usize);
                let mut leq = BTreeSet::new();
                let mut req = BTreeSet::new();
                for (l, r) in lk.iter().zip(&rk) {
                    if let (Some(a), Some(b)) = (col_of(&l.e), col_of(&r.e)) {
                        if crate::sql::value::comparison_affinity(l.affs.0, l.affs.1) == (Aff::None, Aff::None) {
                            leq.insert(a);
                            req.insert(b);
                        }
                    }
                }
                if kr.iter().any(|k| k.is_subset(&req)) {
                    out.extend(kl.iter().cloned());
                }
                if *kind == crate::ir::JoinKind::Inner && kl.iter().any(|k| k.is_subset(&leq)) {
                    out.extend(kr.iter().map(|k| k.iter().map(|c| c + wl).collect()));
                }
                out
            }
            Rel::With { bindings, body } => {
                for (id, b) in bindings {
                    let k = match b {
                        Binding::Plain(r) => self.keys(r),
                        Binding::Fix { set: true, schema, .. } => all(schema.len()),
                        Binding::Fix { .. } => vec![],
                    };
                    self.locals.insert(*id, k);
                }
                self.keys(body)
            }
        }
    }

    /// K1, K6: the order keys' plain columns contain a candidate key of the input (extra
    /// expression keys only refine the order), or cover every input column (ties are then only
    /// between identical rows, which are interchangeable).
    fn total(&mut self, input: &Rel<RepoSource>, cols: &BTreeSet<u16>) -> bool {
        if crate::mutant::on(10) {
            // Mutant 10 (planner audit): every order is taken as proven total.
            return true;
        }
        (0..width(input) as u16).all(|c| cols.contains(&c)) || self.keys(input).iter().any(|k| k.is_subset(cols))
    }

    /// Marks every order-dependent construct `Proven` or `Checked` (decider, 2026-10-22: an
    /// order not proven total is checked at run time, never refused) and records it; refuses
    /// bag recursion (no set fixpoint for the dataflow).
    fn mark(&mut self, r: &mut Rel<RepoSource>, at: &str, marks: &mut Vec<OrderMark>, refusals: &mut Vec<Refusal>, clause: &str) {
        let verdict = |p: bool| if p { Total::Proven } else { Total::Checked };
        match r {
            Rel::Window { input, funcs } => {
                for f in funcs.iter_mut() {
                    if matches!(f.kind, WindowKind::RowNumber) {
                        let cols: BTreeSet<u16> = f.partition.iter().chain(f.order.iter().map(|o| &o.0)).filter_map(col_of).collect();
                        f.total = verdict(self.total(input, &cols));
                        marks.push(OrderMark { at: at.into(), construct: "row_number()".into(), total: f.total });
                    }
                }
            }
            Rel::Sort { input, keys, limit, offset, total } => {
                let cols: BTreeSet<u16> = keys.iter().map(|k| k.0).collect();
                *total = verdict(self.total(input, &cols));
                if limit.is_some() || offset.is_some() {
                    marks.push(OrderMark { at: at.into(), construct: if keys.is_empty() { "LIMIT without ORDER BY".into() } else { "ORDER BY … LIMIT".into() }, total: *total });
                }
            }
            Rel::With { bindings, .. } => {
                for (_, b) in bindings.iter() {
                    if let Binding::Fix { set: false, .. } = b {
                        refusals.push(Refusal { clause: clause.into(), at: at.into(), construct: "order-dependent in a repository plan: bag (UNION ALL) recursion: no set fixpoint for the dataflow".into() });
                    }
                }
            }
            Rel::Aggregate { input, aggs, .. } => {
                if aggs.iter().any(|a| matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray)) {
                    let t = match sorted_input(input) {
                        Some((sin, keys)) => self.total(sin, &keys),
                        None => false,
                    };
                    for a in aggs.iter_mut().filter(|a| matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray)) {
                        a.total = verdict(t);
                        marks.push(OrderMark { at: at.into(), construct: format!("{:?}", a.f), total: a.total });
                    }
                }
            }
            _ => {}
        }
        crate::ir::children_mut(r, &mut |c| self.mark(c, at, marks, refusals, clause));
    }
}

/// An order-dependent construct of a repository plan and whether its order is proven total.
#[derive(Clone, Debug)]
pub struct OrderMark {
    pub at: String,
    pub construct: String,
    pub total: Total,
}

/// Columns a filter pins: conjuncts `col = x` where `x` reads no column of the filtered row and
/// holds no subquery (a literal, or the enclosing row of a correlated subquery), compared with
/// no affinity conversion.
fn pinned_cols<S: Clone>(pred: &Expr<S>) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    for c in crate::ir::conjuncts(pred) {
        if let Expr::Cmp(crate::ir::CmpOp::Eq, a, b, (fa, fb)) = &c {
            if crate::sql::value::comparison_affinity(*fa, *fb) != (Aff::None, Aff::None) {
                continue;
            }
            let constant = |x: &Expr<S>| {
                let mut own = false;
                crate::ir::cols_read(x, &mut |_| own = true);
                !own && !crate::ir::has_subquery(x)
            };
            if let (Some(k), true) = (col_of(a), constant(b)) {
                out.insert(k);
            } else if let (Some(k), true) = (col_of(b), constant(a)) {
                out.insert(k);
            }
        }
    }
    out
}

/// The sort an aggregate's input arrives in (through filters and projections, which keep row
/// order), with its key columns over the sort's own input.
fn sorted_input(r: &Rel<RepoSource>) -> Option<(&Rel<RepoSource>, BTreeSet<u16>)> {
    match r {
        Rel::Sort { input, keys, .. } if !keys.is_empty() => Some((input, keys.iter().map(|k| k.0).collect())),
        Rel::Filter { input, .. } | Rel::Project { input, .. } => sorted_input(input),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The split of a repository clause

struct Cut<'a> {
    cross: &'a [Option<String>],
    /// Per-file stage index of each original stage (file side).
    file_index: &'a BTreeMap<u32, u32>,
    /// Repository stage index of each original stage (repository side).
    repo_index: &'a BTreeMap<u32, u32>,
    /// Per export: its per-content definition, distinct, ordered, and its path columns.
    exports: Vec<(Rel<FileSource>, bool, bool, Vec<u16>)>,
    digests: HashMap<String, ExportId>,
    refusals: Vec<String>,
}

/// Context flags passed down the repository side.
#[derive(Clone, Copy)]
struct Ctx {
    /// Multiplicity is not observable here (a set boundary above).
    set: bool,
    /// Row order is observable here.
    ordered: bool,
}

impl Cut<'_> {
    fn export(&mut self, r: Rel<AnySource>, needed: Option<&BTreeSet<u16>>, ctx: Ctx, path: &[u16]) -> Rel<RepoSource> {
        let schema = r.schema();
        let file_index = self.file_index;
        let def = match to_file(r, self.cross, &|s| file_index[&s]) {
            Ok(d) => d,
            Err(e) => {
                self.refusals.push(e);
                return Rel::Unit;
            }
        };
        let mut keep: Vec<u16> = match needed {
            Some(n) => n.iter().copied().collect(),
            None => (0..schema.len() as u16).collect(),
        };
        if crate::mutant::on(13) && keep.len() > 1 {
            // Mutant 13 (dropped side condition): a needed column is not exported.
            keep.pop();
        }
        let narrow: Vec<_> = keep.iter().map(|c| schema[*c as usize].clone()).collect();
        let mut def = if keep.len() == schema.len() { def } else { Rel::Project { input: Box::new(def), exprs: keep.iter().map(|c| Expr::Col { depth: 0, col: *c }).collect(), schema: narrow.clone() } };
        // Mutant 14 (dropped side condition): distinct even where multiplicity is read.
        let distinct = ctx.set || crate::mutant::on(14);
        if distinct {
            def = Rel::Distinct { input: Box::new(def), prefix: narrow.len() as u16 };
        }
        let pcols: Vec<u16> = path.iter().filter_map(|p| keep.iter().position(|k| k == p).map(|j| j as u16)).collect();
        let digest = format!("{def:?}");
        let id = match self.digests.get(&digest) {
            Some(id) => {
                self.exports[*id as usize].2 |= ctx.ordered;
                *id
            }
            None => {
                self.exports.push((def, distinct, ctx.ordered, pcols));
                let id = (self.exports.len() - 1) as ExportId;
                self.digests.insert(digest, id);
                id
            }
        };
        let scan = Rel::Scan { src: RepoSource::Exported(id), schema: narrow };
        if keep.len() == schema.len() {
            return scan;
        }
        // Re-widen: unread columns are NULL, so the parent's column indices are unchanged.
        let exprs = (0..schema.len() as u16).map(|c| match keep.iter().position(|k| *k == c) {
            Some(j) => Expr::Col { depth: 0, col: j as u16 },
            None => Expr::Lit(crate::ir::Lit::Null),
        });
        Rel::Project { input: Box::new(scan), exprs: exprs.collect(), schema }
    }

    /// The repository side of `r`: maximal file-local subtrees become exports.
    fn rel(&mut self, env: &mut Env<'_>, r: Rel<AnySource>, needed: Option<BTreeSet<u16>>, ctx: Ctx, outer: Option<&[u16]>) -> Rel<RepoSource> {
        if let Loc::Local(path) = env.loc(&r, outer) {
            if !has_free_outer(&r) && self_contained(&r) {
                return self.export(r, needed.as_ref(), ctx, &path);
            }
        }
        let w = width(&r);
        let all = || (0..w as u16).collect::<BTreeSet<u16>>();
        let need = needed.unwrap_or_else(all);
        let sub = |s: &mut Self, env: &mut Env<'_>, e: Expr<AnySource>| s.expr(env, e);
        match r {
            Rel::Scan { src, schema } => match src {
                AnySource::Stage(s) => match self.repo_index.get(&s) {
                    Some(i) => Rel::Scan { src: RepoSource::Stage(*i), schema },
                    None => {
                        // A per-file or constant stage read inside a correlated subtree.
                        self.refusals.push(format!("stage {s} read with an outer correlation from a repository plan"));
                        Rel::Unit
                    }
                },
                AnySource::Input(i) => {
                    self.refusals.push(match &self.cross[i as usize] {
                        Some(c) => format!("{c} in a repository plan (the leaf executor reads one content)"),
                        None => "input read with an outer correlation from a repository plan".into(),
                    });
                    Rel::Unit
                }
                AnySource::Syntax(_) => {
                    self.refusals.push("syntax table read with an outer correlation from a repository plan".into());
                    Rel::Unit
                }
                AnySource::ReferenceEdges => {
                    self.refusals.push("`reference_edges` (the native reference graph) has no definition over exported facts".into());
                    Rel::Unit
                }
            },
            Rel::Local { id, schema } => Rel::Local { id, schema },
            Rel::Unit => Rel::Unit,
            Rel::Filter { input, pred } => {
                let mut n = need;
                expr_cols(&pred, 0, &mut n);
                let input = self.rel(env, *input, Some(n), ctx, outer);
                Rel::Filter { input: Box::new(input), pred: sub(self, env, pred) }
            }
            Rel::Project { input, exprs, schema } => {
                let mut n = BTreeSet::new();
                for (j, e) in exprs.iter().enumerate() {
                    if need.contains(&(j as u16)) || has_subquery_or_side(e) {
                        expr_cols(e, 0, &mut n);
                    }
                }
                let input = self.rel(env, *input, Some(n), ctx, outer);
                let exprs = exprs.into_iter().map(|e| sub(self, env, e)).collect();
                Rel::Project { input: Box::new(input), exprs, schema }
            }
            Rel::Join { kind, left, right, on } => {
                let wl = width(&left) as u16;
                let mut n = need;
                if let Some(o) = &on {
                    expr_cols(o, 0, &mut n);
                }
                let nl = n.iter().filter(|c| **c < wl).copied().collect();
                let nr = n.iter().filter(|c| **c >= wl).map(|c| c - wl).collect();
                let left = self.rel(env, *left, Some(nl), ctx, outer);
                let right = self.rel(env, *right, Some(nr), ctx, outer);
                Rel::Join { kind, left: Box::new(left), right: Box::new(right), on: on.map(|o| sub(self, env, o)) }
            }
            Rel::JsonEach { input, arg, on, outer: o } => {
                let wi = width(&input) as u16;
                let mut n: BTreeSet<u16> = need.into_iter().filter(|c| *c < wi).collect();
                expr_cols(&arg, 0, &mut n);
                if let Some(x) = &on {
                    let mut m = BTreeSet::new();
                    expr_cols(x, 0, &mut m);
                    n.extend(m.into_iter().filter(|c| *c < wi));
                }
                let input = self.rel(env, *input, Some(n), ctx, outer);
                Rel::JsonEach { input: Box::new(input), arg: sub(self, env, arg), on: on.map(|x| sub(self, env, x)), outer: o }
            }
            Rel::Aggregate { input, keys, aggs } => {
                let wi = width(&input) as u16;
                let mut n: BTreeSet<u16> = need.iter().copied().filter(|c| *c < wi).collect();
                for k in &keys {
                    expr_cols(k, 0, &mut n);
                }
                for a in &aggs {
                    a.arg.iter().chain(a.sep.iter()).for_each(|e| expr_cols(e, 0, &mut n));
                }
                // Bare columns read the group's representative row (the last row, or the lone
                // min/max row): order-dependent unless they are group keys.
                let key_cols: BTreeSet<u16> = keys.iter().filter_map(col_of).collect();
                let bare = need.iter().any(|c| *c < wi && !key_cols.contains(c));
                let order_aggs = aggs.iter().any(|a| matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray));
                if bare {
                    self.refusals.push("order-dependent in a repository plan: aggregate bare column (the group's last or min/max row)".into());
                }
                let set_ok = aggs.iter().all(|a| matches!(a.f, AggFn::Min | AggFn::Max) || a.distinct && matches!(a.f, AggFn::Count));
                let c = Ctx { set: set_ok && !bare, ordered: ctx.ordered || order_aggs || bare };
                let input = self.rel(env, *input, Some(n), c, outer);
                let keys = keys.into_iter().map(|e| sub(self, env, e)).collect();
                let aggs = aggs
                    .into_iter()
                    .map(|a| crate::ir::AggCall { f: a.f, arg: a.arg.map(|e| sub(self, env, e)), sep: a.sep.map(|e| sub(self, env, e)), distinct: a.distinct, total: a.total })
                    .collect();
                Rel::Aggregate { input: Box::new(input), keys, aggs }
            }
            Rel::Window { input, funcs } => {
                let wi = width(&input) as u16;
                let mut n: BTreeSet<u16> = need.iter().copied().filter(|c| *c < wi).collect();
                for f in &funcs {
                    f.partition.iter().chain(f.order.iter().map(|o| &o.0)).for_each(|e| expr_cols(e, 0, &mut n));
                    if let WindowKind::Min(e) | WindowKind::Max(e) = &f.kind {
                        expr_cols(e, 0, &mut n);
                    }
                }
                let row_number = funcs.iter().any(|f| matches!(f.kind, WindowKind::RowNumber));
                let c = Ctx { set: false, ordered: ctx.ordered || row_number };
                let input = self.rel(env, *input, Some(n), c, outer);
                let funcs = funcs
                    .into_iter()
                    .map(|f| crate::ir::WindowFn {
                        kind: match f.kind {
                            WindowKind::RowNumber => WindowKind::RowNumber,
                            WindowKind::Min(e) => WindowKind::Min(sub(self, env, e)),
                            WindowKind::Max(e) => WindowKind::Max(sub(self, env, e)),
                        },
                        partition: f.partition.into_iter().map(|e| sub(self, env, e)).collect(),
                        order: f.order.into_iter().map(|(e, d)| (sub(self, env, e), d)).collect(),
                        total: f.total,
                    })
                    .collect();
                Rel::Window { input: Box::new(input), funcs }
            }
            Rel::Distinct { input, prefix } => {
                let wi = width(&input);
                let n: BTreeSet<u16> = need.into_iter().chain(0..prefix).collect();
                let c = Ctx { set: (prefix as usize) == wi || n.iter().all(|c| *c < prefix), ordered: ctx.ordered || (prefix as usize) < wi };
                let input = self.rel(env, *input, Some(n), c, outer);
                Rel::Distinct { input: Box::new(input), prefix }
            }
            Rel::Union { all, left, right } => {
                let n = if all { Some(need) } else { None };
                let c = Ctx { set: ctx.set || !all, ordered: ctx.ordered };
                let left = self.rel(env, *left, n.clone(), c, outer);
                let right = self.rel(env, *right, n, c, outer);
                Rel::Union { all, left: Box::new(left), right: Box::new(right) }
            }
            Rel::Sort { input, keys, limit, offset, total } => {
                let n: BTreeSet<u16> = need.into_iter().chain(keys.iter().map(|k| k.0)).collect();
                let limited = limit.is_some() || offset.is_some();
                let c = Ctx { set: ctx.set && !limited, ordered: ctx.ordered || limited };
                let input = self.rel(env, *input, Some(n), c, outer);
                Rel::Sort { input: Box::new(input), keys, limit: limit.map(|e| sub(self, env, e)), offset: offset.map(|e| sub(self, env, e)), total }
            }
            Rel::With { bindings, body } => {
                let mut out = Vec::new();
                for (id, b) in bindings {
                    let l = env.binding(id, &b, outer);
                    let nb = match b {
                        Binding::Plain(r) => Binding::Plain(self.rel(env, r, None, Ctx { set: false, ordered: true }, outer)),
                        Binding::Fix { init, step, set, schema } => {
                            let c = Ctx { set, ordered: !set };
                            Binding::Fix {
                                init: init.into_iter().map(|r| self.rel(env, r, None, c, outer)).collect(),
                                step: step.into_iter().map(|r| self.rel(env, r, None, c, outer)).collect(),
                                set,
                                schema,
                            }
                        }
                    };
                    env.locals.insert(id, l);
                    out.push((id, nb));
                }
                let body = self.rel(env, *body, Some(need), ctx, outer);
                Rel::With { bindings: out, body: Box::new(body) }
            }
        }
    }

    fn expr(&mut self, env: &mut Env<'_>, e: Expr<AnySource>) -> Expr<RepoSource> {
        if let Expr::Func(Func::SourceText, _) = &e {
            self.refusals.push("source_text in a repository plan (another content's source)".into());
        }
        // Subqueries: their relations are cut in the same way, with outer correlation kept.
        map_expr_rel(e, &mut |rel, kind| {
            let ctx = match kind {
                SubKind::Exists | SubKind::In => Ctx { set: true, ordered: false },
                SubKind::Scalar => Ctx { set: false, ordered: true },
            };
            self.rel(env, rel, None, ctx, None)
        })
    }
}

fn has_subquery_or_side<S>(e: &Expr<S>) -> bool {
    let mut f = false;
    subqueries(e, &mut |_| f = true);
    f
}

enum SubKind {
    Exists,
    In,
    Scalar,
}

/// Maps an expression's subquery relations with `f`, everything else structurally.
fn map_expr_rel(e: Expr<AnySource>, f: &mut dyn FnMut(Rel<AnySource>, SubKind) -> Rel<RepoSource>) -> Expr<RepoSource> {
    let b = |x: Box<Expr<AnySource>>, f: &mut dyn FnMut(Rel<AnySource>, SubKind) -> Rel<RepoSource>| Box::new(map_expr_rel(*x, f));
    match e {
        Expr::Lit(l) => Expr::Lit(l),
        Expr::Col { depth, col } => Expr::Col { depth, col },
        Expr::Not(x) => Expr::Not(b(x, f)),
        Expr::Neg(x) => Expr::Neg(b(x, f)),
        Expr::Plus(x) => Expr::Plus(b(x, f)),
        Expr::Cast(x, a) => Expr::Cast(b(x, f), a),
        Expr::Arith(o, x, y) => Expr::Arith(o, b(x, f), b(y, f)),
        Expr::Concat(x, y) => Expr::Concat(b(x, f), b(y, f)),
        Expr::And(x, y) => Expr::And(b(x, f), b(y, f)),
        Expr::Or(x, y) => Expr::Or(b(x, f), b(y, f)),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(o, b(x, f), b(y, f), a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left, f), right: b(right, f), negated },
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e, f), negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: b(text, f), pattern: b(pattern, f), negated },
        Expr::Func(fu, args) => Expr::Func(fu, args.into_iter().map(|a| map_expr_rel(a, f)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case {
            operand: operand.map(|o| b(o, f)),
            whens: whens.into_iter().map(|(w, t, a)| (map_expr_rel(w, f), map_expr_rel(t, f), a)).collect(),
            else_: else_.map(|x| b(x, f)),
        },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e, f), list: list.into_iter().map(|(x, a)| (map_expr_rel(x, f), a)).collect(), negated },
        Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: b(e, f), rel: Box::new(f(*rel, SubKind::In)), negated, aff },
        Expr::Exists { rel, negated } => Expr::Exists { rel: Box::new(f(*rel, SubKind::Exists)), negated },
        Expr::Scalar(rel) => Expr::Scalar(Box::new(f(*rel, SubKind::Scalar))),
    }
}

// ---------------------------------------------------------------------------------------------
// Program assembly

/// Per clause, what the classifier decided (for the census).
#[derive(Clone, Debug)]
pub struct ClauseReport {
    pub clause: String,
    pub form: &'static str,
    /// `relational.partitionBy`; `None` for `forbid`/`budget` (implicitly per file).
    pub declared: Option<Partition>,
    /// The class inferred from the leaves; `None` when the clause was refused before.
    pub inferred: Option<Partition>,
    pub file_stages: usize,
    pub repo_stages: usize,
    pub exports: Vec<ExportId>,
    /// Order-dependent constructs of the repository plan, `Proven` or `Checked`.
    pub order: Vec<OrderMark>,
}

pub struct Classified {
    pub per_file: Vec<Plan<FileSource>>,
    pub repository: Vec<Plan<RepoSource>>,
    pub exported: Vec<ExportedRelation>,
    pub refusals: Vec<Refusal>,
    pub reports: Vec<ClauseReport>,
}

fn stage_reads<S>(st: &Stage<S>, out: &mut BTreeSet<u32>, pick: &dyn Fn(&S) -> Option<u32>) {
    fn walk<S>(r: &Rel<S>, out: &mut BTreeSet<u32>, pick: &dyn Fn(&S) -> Option<u32>) {
        if let Rel::Scan { src, .. } = r {
            if let Some(s) = pick(src) {
                out.insert(s);
            }
        }
        each_expr(r, &mut |e| subqueries(e, &mut |s| walk(s, out, pick)));
        each_child(r, &mut |c| walk(c, out, pick));
    }
    match &st.def {
        StageDef::Query { rel } => walk(rel, out, pick),
        StageDef::Reach { roots, edges, .. } => {
            walk(roots, out, pick);
            if let Some(e) = edges {
                walk(e, out, pick);
            }
        }
    }
}

pub fn classify(front: Front) -> Classified {
    let Front { inputs, input_cross, clauses, .. } = front;
    let cross = &input_cross;
    let mut out = Classified { per_file: vec![], repository: vec![], exported: vec![], refusals: vec![], reports: vec![] };
    for c in clauses {
        let Some(body) = c.body else { continue };
        let id = c.id.clone();
        let refuse = |at: &str, construct: String, out: &mut Classified| out.refusals.push(Refusal { clause: id.clone(), at: at.into(), construct });
        match body {
            ClauseBody::Forbid { input, violation } => {
                let inferred = if cross[input as usize].is_some() { Partition::Repository } else { Partition::File };
                out.reports.push(ClauseReport { clause: c.id.clone(), form: "forbid", declared: None, inferred: Some(inferred), file_stages: 0, repo_stages: 0, exports: vec![], order: vec![] });
                match file_source(AnySource::Input(input), cross) {
                    Ok(_) => out.per_file.push(Plan { clause: c.id, syntax: None, stages: vec![], sink: Sink::Witnesses { input, violation } }),
                    Err(e) => refuse("forbid", format!("{e}; a forbid clause is per file"), &mut out),
                }
            }
            ClauseBody::Budget { input, closure, condition, group_by, distinct, lte, violation } => {
                // Per file iff a group never spans files: group_by and every closure partition
                // hold a field projected from `file.path`.
                let path_fields: Vec<&str> = inputs[input as usize].fields.iter().filter(|(_, p)| *p == Projection::Path).map(|(f, _)| f.as_str()).collect();
                let has = |fields: &[String]| fields.iter().any(|f| path_fields.contains(&f.as_str()));
                let per_file = has(&group_by) && closure.iter().all(|s| has(&s.partition_by));
                let declared = None;
                let schema = inputs[input as usize].schema.clone();
                let scan = Rel::Scan { src: AnySource::Input(input), schema: schema.clone() };
                if per_file {
                    out.reports.push(ClauseReport { clause: c.id.clone(), form: "budget", declared, inferred: Some(Partition::File), file_stages: 0, repo_stages: 0, exports: vec![], order: vec![] });
                    match to_file(scan, cross, &|s| s) {
                        Ok(rel) => out.per_file.push(Plan { clause: c.id, syntax: None, stages: vec![], sink: Sink::Budget { rel, closure, condition, group_by, distinct, lte, violation } }),
                        Err(e) => refuse("budget", e, &mut out),
                    }
                } else {
                    let exp = out.exported.len() as ExportId;
                    let producer = out.per_file.len();
                    match to_file(scan, cross, &|s| s) {
                        Err(e) => {
                            refuse("budget", format!("{e} (a budget spanning files exports its rows; this input cannot be computed per content)"), &mut out);
                            out.reports.push(ClauseReport { clause: c.id.clone(), form: "budget", declared, inferred: Some(Partition::Repository), file_stages: 0, repo_stages: 0, exports: vec![], order: vec![] });
                        }
                        Ok(rel) => {
                            out.per_file.push(Plan { clause: c.id.clone(), syntax: None, stages: vec![], sink: Sink::Export(vec![(exp, rel)]) });
                            out.exported.push(ExportedRelation { id: exp, name: format!("{}/rows", c.id), schema: schema.clone(), distinct: false, ordered: true, producer });
                            out.repository.push(Plan { clause: c.id.clone(), syntax: None, stages: vec![], sink: Sink::Budget { rel: Rel::Scan { src: RepoSource::Exported(exp), schema }, closure, condition, group_by, distinct, lte, violation } });
                            out.reports.push(ClauseReport { clause: c.id.clone(), form: "budget", declared, inferred: Some(Partition::Repository), file_stages: 0, repo_stages: 0, exports: vec![exp], order: vec![] });
                        }
                    }
                }
            }
            ClauseBody::Relational { syntax, stages, fin, key, code, locations, violation } => {
                let declared = c.declared.unwrap_or(Partition::Repository);
                if declared == Partition::File {
                    let mut ok = true;
                    let mut fstages = Vec::new();
                    for st in stages {
                        let name = st.name.clone();
                        match stage_to_file(st, cross, &|s| s) {
                            Ok(s) => fstages.push(s),
                            Err(e) => {
                                refuse(&format!("stages/{name}"), e, &mut out);
                                ok = false;
                            }
                        }
                    }
                    let fin = match to_file(fin, cross, &|s| s) {
                        Ok(f) => Some(f),
                        Err(e) => {
                            refuse("query", e, &mut out);
                            None
                        }
                    };
                        out.reports.push(ClauseReport {
                        clause: c.id.clone(),
                        form: "relational",
                        declared: Some(declared),
                        inferred: Some(if ok && fin.is_some() { Partition::File } else { Partition::Repository }),
                        file_stages: fstages.len(),
                        repo_stages: 0,
                        exports: vec![],
                        order: vec![],
                    });
                    if let (true, Some(fin)) = (ok, fin) {
                        out.per_file.push(Plan { clause: c.id, syntax, stages: fstages, sink: Sink::Findings { rel: fin, key, code, locations, violation } });
                    }
                    continue;
                }
                split(&mut out, &c.id, cross, syntax, stages, fin, key, code, locations, violation);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn split(
    out: &mut Classified,
    clause: &str,
    cross: &[Option<String>],
    syntax: Option<crate::ir::SyntaxSpec>,
    stages: Vec<Stage<AnySource>>,
    fin: Rel<AnySource>,
    key: Vec<String>,
    code: Option<String>,
    locations: Vec<String>,
    violation: crate::ir::Violation,
) {
    let refuse = |out: &mut Classified, at: &str, construct: String| out.refusals.push(Refusal { clause: clause.into(), at: at.into(), construct });
    let mut env = Env { cross, stages: vec![], locals: HashMap::new() };
    for st in &stages {
        let l = match &st.def {
            StageDef::Query { rel } => env.loc(rel, None),
            StageDef::Reach { roots, edges, .. } => {
                let lr = env.loc(roots, None);
                let le = edges.as_ref().map(|e| env.loc(e, None)).unwrap_or(Loc::Const);
                let local = |l: &Loc| matches!(l, Loc::Const) || matches!(l, Loc::Local(p) if p.contains(&0));
                if local(&lr) && local(&le) && !matches!((&lr, &le), (Loc::Const, Loc::Const)) { Loc::Local(vec![0]) } else if matches!((&lr, &le), (Loc::Const, Loc::Const)) { Loc::Const } else { Loc::Global }
            }
        };
        env.stages.push(l);
    }
    // Which stages each side reads.
    let pick = |s: &AnySource| if let AnySource::Stage(i) = s { Some(*i) } else { None };
    let is_file = |l: &Loc| matches!(l, Loc::Local(_));
    let mut repo_needed: BTreeSet<u32> = BTreeSet::new();
    let mut file_needed: BTreeSet<u32> = BTreeSet::new();
    let fin_stage = Stage { name: "query".into(), schema: vec![], def: StageDef::Query { rel: fin } };
    {
        let mut reads = BTreeSet::new();
        stage_reads(&fin_stage, &mut reads, &pick);
        repo_needed.extend(reads);
    }
    for (i, st) in stages.iter().enumerate().rev() {
        let i = i as u32;
        let mut reads = BTreeSet::new();
        stage_reads(st, &mut reads, &pick);
        if is_file(&env.stages[i as usize]) {
            file_needed.insert(i);
        }
        if file_needed.contains(&i) {
            file_needed.extend(reads.iter().copied());
        }
        if repo_needed.contains(&i) && !is_file(&env.stages[i as usize]) {
            repo_needed.extend(reads.iter().copied());
        }
    }
    // The file side first takes every per-content and constant stage; exports discovered by the
    // cut may read any of them. Unread ones are pruned after the cut.
    let _ = &file_needed;
    let file_ids: Vec<u32> = (0..stages.len() as u32).filter(|i| matches!(env.stages[*i as usize], Loc::Local(_) | Loc::Const)).collect();
    let repo_ids: Vec<u32> = (0..stages.len() as u32).filter(|i| repo_needed.contains(i) && !is_file(&env.stages[*i as usize])).collect();
    let file_index: BTreeMap<u32, u32> = file_ids.iter().enumerate().map(|(j, i)| (*i, j as u32)).collect();
    let repo_index: BTreeMap<u32, u32> = repo_ids.iter().enumerate().map(|(j, i)| (*i, j as u32)).collect();
    let Stage { def: StageDef::Query { rel: fin }, .. } = fin_stage else { unreachable!() };
    let mut cut = Cut { cross, file_index: &file_index, repo_index: &repo_index, exports: vec![], digests: HashMap::new(), refusals: vec![] };
    let mut fstages = Vec::new();
    let mut rstages = Vec::new();
    let mut stages: Vec<Option<Stage<AnySource>>> = stages.into_iter().map(Some).collect();
    for i in 0..stages.len() as u32 {
        let in_file = file_index.contains_key(&i);
        let in_repo = repo_index.contains_key(&i);
        let st = if in_file && in_repo { stages[i as usize].clone() } else { stages[i as usize].take() };
        let Some(st) = st else { continue };
        if in_file {
            match stage_to_file(st.clone(), cross, &|s| file_index[&s]) {
                Ok(s) => fstages.push(s),
                Err(e) => refuse(out, &format!("stages/{}", st.name), e),
            }
        }
        if in_repo {
            let full = Ctx { set: false, ordered: true };
            let def = match st.def {
                StageDef::Query { rel } => StageDef::Query { rel: cut.rel(&mut env, rel, None, full, None) },
                StageDef::Reach { roots, edges, graph } => StageDef::Reach {
                    roots: cut.rel(&mut env, roots, None, Ctx { set: true, ordered: false }, None),
                    edges: edges.map(|e| cut.rel(&mut env, e, None, Ctx { set: true, ordered: false }, None)),
                    graph,
                },
            };
            rstages.push(Stage { name: st.name, schema: st.schema, def });
        }
    }
    let fin = cut.rel(&mut env, fin, None, Ctx { set: false, ordered: false }, None);
    for r in std::mem::take(&mut cut.refusals) {
        refuse(out, "repository", r);
    }
    // Prune file-side stages no export reads (transitively), and renumber.
    let fpick = |s: &FileSource| if let FileSource::Stage(i) = s { Some(*i) } else { None };
    let mut live: BTreeSet<u32> = BTreeSet::new();
    for (def, ..) in &cut.exports {
        let probe = Stage { name: String::new(), schema: vec![], def: StageDef::Query { rel: def.clone() } };
        stage_reads(&probe, &mut live, &fpick);
    }
    for j in (0..fstages.len() as u32).rev() {
        if live.contains(&j) {
            let mut reads = BTreeSet::new();
            stage_reads(&fstages[j as usize], &mut reads, &fpick);
            live.extend(reads);
        }
    }
    let remap: BTreeMap<u32, u32> = live.iter().enumerate().map(|(k, j)| (*j, k as u32)).collect();
    let refile = |r: Rel<FileSource>| -> Rel<FileSource> { map_rel(r, &mut |s| Ok::<_, ()>(match s { FileSource::Stage(i) => FileSource::Stage(remap[&i]), o => o })).unwrap() };
    let fstages: Vec<Stage<FileSource>> = fstages
        .into_iter()
        .enumerate()
        .filter(|(j, _)| live.contains(&(*j as u32)))
        .map(|(_, st)| Stage {
            name: st.name,
            schema: st.schema,
            def: match st.def {
                StageDef::Query { rel } => StageDef::Query { rel: refile(rel) },
                StageDef::Reach { roots, edges, graph } => StageDef::Reach { roots: refile(roots), edges: edges.map(refile), graph },
            },
        })
        .collect();
    cut.exports = std::mem::take(&mut cut.exports).into_iter().map(|(d, a, b, p)| (refile(d), a, b, p)).collect();
    // Candidate keys of each export over the union of contents: a per-content key plus a path
    // column (rows of different contents differ there); without a path column, none.
    let mut fkeys = KeyEnv { stages: vec![], locals: HashMap::new(), exports: vec![] };
    for st in &fstages {
        let k = match &st.def {
            StageDef::Query { rel } => fkeys.keys(rel),
            StageDef::Reach { .. } => vec![(0..3).collect()],
        };
        fkeys.stages.push(k);
    }
    let export_keys: Vec<Keys> = cut
        .exports
        .iter()
        .map(|(def, _, _, pcols)| match pcols.first() {
            None => vec![],
            Some(p) => fkeys.keys(def).into_iter().map(|mut k| {
                k.insert(*p);
                k
            }).collect(),
        })
        .collect();
    let producer = out.per_file.len();
    let base = out.exported.len() as ExportId;
    let mut export_ids = Vec::new();
    let mut sinks = Vec::new();
    let mut all_export_keys: Vec<Keys> = out.exported.iter().map(|_| vec![]).collect();
    for (k, (def, distinct, ordered, _)) in cut.exports.into_iter().enumerate() {
        all_export_keys.push(export_keys[k].clone());
        let id = base + k as ExportId;
        out.exported.push(ExportedRelation { id, name: format!("{clause}/export_{k}"), schema: def.schema(), distinct, ordered, producer });
        export_ids.push(id);
        sinks.push((id, def));
    }
    // Renumber the repository side's export scans to the program-wide ids.
    let renumber = |r: Rel<RepoSource>| -> Rel<RepoSource> { map_rel(r, &mut |s| Ok::<_, ()>(match s { RepoSource::Exported(e) => RepoSource::Exported(base + e), o => o })).unwrap() };
    let rstages: Vec<Stage<RepoSource>> = rstages
        .into_iter()
        .map(|st| Stage {
            name: st.name,
            schema: st.schema,
            def: match st.def {
                StageDef::Query { rel } => StageDef::Query { rel: renumber(rel) },
                StageDef::Reach { roots, edges, graph } => StageDef::Reach { roots: renumber(roots), edges: edges.map(renumber), graph },
            },
        })
        .collect();
    let fin = renumber(fin);
    // Order audit over the repository side.
    let mut kenv = KeyEnv { stages: vec![], locals: HashMap::new(), exports: all_export_keys };
    let (mut marks, mut audit) = (Vec::new(), Vec::new());
    let mut rstages = rstages;
    for st in &mut rstages {
        match &mut st.def {
            StageDef::Query { rel } => {
                kenv.mark(rel, &format!("stages/{}", st.name), &mut marks, &mut audit, clause);
                let k = kenv.keys(rel);
                kenv.stages.push(k);
            }
            StageDef::Reach { .. } => kenv.stages.push(vec![(0..3).collect()]),
        }
    }
    let mut fin = fin;
    kenv.mark(&mut fin, "query", &mut marks, &mut audit, clause);
    out.refusals.extend(audit);
    let (nf, nr) = (fstages.len(), rstages.len());
    out.reports.push(ClauseReport { clause: clause.into(), form: "relational", declared: Some(Partition::Repository), inferred: Some(Partition::Repository), file_stages: nf, repo_stages: nr, exports: export_ids, order: marks });
    out.per_file.push(Plan { clause: clause.into(), syntax, stages: fstages, sink: Sink::Export(sinks) });
    out.repository.push(Plan { clause: clause.into(), syntax: None, stages: rstages, sink: Sink::Findings { rel: fin, key, code, locations, violation } });
}

#[cfg(test)]
mod tests {
    use crate::compile::compile;
    use crate::front::rule::LeafCheck;
    use crate::ir::{Partition, Rel, RepoSource, Sink, StageDef};
    use serde_json::{Value as Json, json};
    use std::collections::BTreeMap;

    struct Accept;
    impl LeafCheck for Accept {
        fn pattern(&mut self, _: &Json, _: &BTreeMap<String, Json>) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn regex(&mut self, _: &Json) -> Result<(), String> {
            Ok(())
        }
    }

    fn evidence(project: Json) -> Json {
        json!({ "evidence": { "as": "row", "query": { "ast": { "language": "python", "rule": { "kind": "identifier" } } }, "project": project } })
    }

    fn policy(clause: Json) -> Json {
        json!({ "format": "rerail.policy/v1", "id": "leak-test", "revision": 1, "mode": "report", "clauses": [clause] })
    }

    fn relational(partition: Json, extra: Json, stage: &str) -> Json {
        let mut r = json!({
            "partitionBy": partition,
            "inputs": { "names": evidence(json!({ "name": "capture.N" })) },
            "stages": [{ "name": "s", "query": stage }],
            "query": "SELECT __path AS path, __start AS start_byte, __end AS end_byte FROM s",
        });
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        policy(json!({ "id": "c", "relational": r, "violation": { "code": "X", "message": "m", "anchor": "witness.span" } }))
    }

    fn refused(doc: &Json) -> Vec<String> {
        match compile(doc, &Default::default(), &mut Accept) {
            Ok(_) => vec![],
            Err(u) => u.refusals.into_iter().map(|r| r.construct).collect(),
        }
    }

    #[test]
    fn leak_control_per_file_clause_compiles() {
        let doc = relational(json!("file"), json!({}), "SELECT * FROM names");
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        assert_eq!(p.per_file.len(), 1);
        assert!(p.repository.is_empty() && p.exported.is_empty());
    }

    /// The seeded cross-file leak: a `partitionBy: file` clause reading the reference graph.
    #[test]
    fn leak_reference_edges_in_per_file_plan_is_rejected() {
        let doc = relational(
            json!("file"),
            json!({ "references": { "language": "python" } }),
            "SELECT n.* FROM names n JOIN reference_edges e ON e.target_name = n.name",
        );
        let r = refused(&doc);
        assert!(r.iter().any(|c| c.contains("cross-file read in a per-file plan") && c.contains("reference_edges")), "{r:?}");
    }

    #[test]
    fn leak_calls_atom_in_forbid_is_rejected() {
        let doc = policy(json!({ "id": "c", "forbid": { "ast": { "language": "python", "rule": { "all": [{ "kind": "function_definition" }, { "calls": { "graph": "references" } }] } } }, "violation": { "code": "X", "message": "m", "anchor": "witness.span" } }));
        let r = refused(&doc);
        assert!(r.iter().any(|c| c.contains("cross-file read in a per-file plan") && c.contains("calls")), "{r:?}");
    }

    #[test]
    fn leak_module_projection_in_per_file_plan_is_rejected() {
        let mut doc = relational(json!("file"), json!({}), "SELECT * FROM names");
        doc["clauses"][0]["relational"]["inputs"]["names"] = evidence(json!({ "name": "capture.N", "module": "file.module" }));
        let r = refused(&doc);
        assert!(r.iter().any(|c| c.contains("cross-file read in a per-file plan") && c.contains("file.module")), "{r:?}");
    }

    /// A repository clause joining names across files: the per-file side exports the names
    /// (projected, distinct: the join reads them through EXISTS), the repository plan scans only
    /// exported relations.
    #[test]
    fn repository_clause_reads_only_exports() {
        let doc = relational(
            json!(null),
            json!({}),
            "SELECT a.__path, a.__start, a.__end FROM names a WHERE EXISTS (SELECT 1 FROM names b WHERE b.name = a.name AND b.__path <> a.__path)",
        );
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        assert_eq!(p.repository.len(), 1);
        assert!(!p.exported.is_empty());
        fn scans(r: &Rel<RepoSource>, out: &mut Vec<RepoSource>) {
            if let Rel::Scan { src, .. } = r {
                out.push(src.clone());
            }
            crate::census::visit_children(r, &mut |c| scans(c, out));
        }
        let mut found = Vec::new();
        for st in &p.repository[0].stages {
            if let StageDef::Query { rel } = &st.def {
                scans(rel, &mut found);
            }
        }
        if let Sink::Findings { rel, .. } = &p.repository[0].sink {
            scans(rel, &mut found);
        }
        assert!(found.iter().all(|s| matches!(s, RepoSource::Exported(_) | RepoSource::Stage(_))), "{found:?}");
        assert!(p.exported.iter().any(|e| e.distinct), "the EXISTS side is read as a set");
        let _ = Partition::File;
    }

    /// Order proofs on repository plans (K1, K2, K4, K6) mark `Proven`; an order without a
    /// proof compiles marked `Checked` (mutant 10 proves everything).
    #[test]
    fn order_marks_proven_and_checked() {
        use crate::ir::Total;
        // A cross-file selection: the repository side.
        let x = "(SELECT a.__path AS p, a.__start AS s, a.__end AS e, a.name AS n FROM names a WHERE EXISTS (SELECT 1 FROM names b WHERE b.name = a.name AND b.__path <> a.__path))";
        let d = format!("(SELECT DISTINCT p, n FROM {x})");
        let cases: Vec<(String, Total)> = vec![
            // K6: partition + order cover every input column.
            (format!("SELECT p AS __path, s AS __start, e AS __end, row_number() OVER (PARTITION BY p ORDER BY n, s, e) AS rn FROM {x}"), Total::Proven),
            // Not total: ties on `n` between distinct rows are possible.
            (format!("SELECT p AS __path, s AS __start, e AS __end, row_number() OVER (PARTITION BY p ORDER BY n) AS rn FROM {x}"), Total::Checked),
            (format!("SELECT p AS __path, s AS __start, e AS __end FROM {x} ORDER BY n LIMIT 1"), Total::Checked),
            // K1: the plain columns (p, n) are the DISTINCT key; `length(n)` only refines.
            (format!("SELECT p AS __path, 0 AS __start, 0 AS __end, row_number() OVER (PARTITION BY p ORDER BY length(n), n) AS rn FROM {d}"), Total::Proven),
            // K4: `rn = 1` over row_number() partitioned by p leaves p a key.
            (format!("SELECT p AS __path, 0 AS __start, 0 AS __end FROM (SELECT p, n, row_number() OVER (PARTITION BY p ORDER BY n) AS rn FROM {d}) WHERE rn = 1 ORDER BY p LIMIT 5"), Total::Proven),
            // K2: the right side's key (n) is equated to the left: the left key (p, n) stays.
            (format!("SELECT l.p AS __path, 0 AS __start, 0 AS __end FROM {d} l JOIN (SELECT DISTINCT n FROM {x}) r ON r.n = l.n ORDER BY l.p, l.n LIMIT 3"), Total::Proven),
            // K3: inside a correlated subquery, `d2.p = l.p` pins p, so (n) orders it totally.
            (format!("SELECT l.p AS __path, 0 AS __start, 0 AS __end, (SELECT d2.n FROM {d} d2 WHERE d2.p = l.p ORDER BY d2.n LIMIT 1) AS first FROM {d} l"), Total::Proven),
        ];
        for (k, (stage, want)) in cases.iter().enumerate() {
            let doc = relational(json!(null), json!({}), stage);
            let c = crate::compile::compile_report(&doc, &Default::default(), &mut Accept).unwrap();
            assert!(c.refusals.is_empty(), "case {k}: {:?}", c.refusals);
            let marks: Vec<_> = c.reports.iter().flat_map(|r| r.order.iter()).collect();
            assert!(!marks.is_empty(), "case {k}: no order construct marked");
            // The case's construct is the last one marked (K4's inner row_number is not total).
            assert_eq!(marks.last().unwrap().total, *want, "case {k}: {marks:?}");
            let mut found = false;
            for p in &c.program.repository {
                for st in &p.stages {
                    if let StageDef::Query { rel } = &st.def {
                        found |= format!("{rel:?}").contains(&format!("total: {want:?}"));
                    }
                }
            }
            assert!(found, "case {k}: the IR node carries the mark");
        }
    }

    /// Mutant 15: a join of two per-content relations without a path equality spans files.
    #[test]
    fn rewrite_cut_requires_path_equality() {
        let doc = relational(json!(null), json!({}), "SELECT a.__path, a.__start, a.__end FROM names a JOIN names b ON b.name = a.name");
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        // The stage itself is not exported: two scans of the names export feed a repository join.
        assert_eq!(p.repository[0].stages.len(), 1, "the join stays on the repository side");
    }

    /// Mutant 14: a count reads multiplicity, so its export is not distinct.
    #[test]
    fn rewrite_distinct_only_under_set_readers() {
        let doc = relational(json!(null), json!({}), "SELECT n.__path, n.__start, n.__end FROM names n WHERE (SELECT count(*) FROM names m WHERE m.name = n.name) > 1");
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        assert!(!p.exported.is_empty() && p.exported.iter().all(|e| !e.distinct), "{:?}", p.exported.iter().map(|e| (&e.name, e.distinct)).collect::<Vec<_>>());
    }

    /// Mutant 13: an export keeps every column the repository side reads.
    #[test]
    fn rewrite_projection_keeps_read_columns() {
        let doc = relational(json!(null), json!({}), "SELECT a.__path, a.__start, a.__end, a.name FROM names a JOIN names b ON b.name = a.name");
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        for e in &p.exported {
            let cols: Vec<&str> = e.schema.iter().map(|c| c.name.as_str()).collect();
            assert!(cols.contains(&"name"), "{cols:?}");
        }
    }

    /// Mutant 12: a deferred column reads the stored column it names.
    #[test]
    fn rewrite_deferred_reads_stored_columns() {
        let mut doc = relational(json!("file"), json!({}), "SELECT name, __path, __start, __end FROM names");
        doc["clauses"][0]["relational"]["stages"][0]["deferred"] = json!({ "name": "__path || ':' " });
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        let StageDef::Query { rel: Rel::Project { exprs, .. } } = &p.per_file[0].stages[0].def else { panic!() };
        // Stored columns: __path (query column 1), __start, __end; `name` (column 0) is replaced.
        let crate::ir::Expr::Concat(l, _) = &exprs[0] else { panic!("{:?}", exprs[0]) };
        assert!(matches!(**l, crate::ir::Expr::Col { depth: 0, col: 1 }), "{l:?}");
    }

    /// Mutant 11: `file: {path: C}` becomes a `filePath: C` conjunct, C whole; without `syntax`
    /// the gate stays (and the leaf executor decides).
    #[test]
    fn rewrite_file_path_gate() {
        let q = json!({ "language": "python", "syntax": "valid", "file": { "path": { "regex": "^impl/", "notRegex": "tests/" } }, "rule": { "kind": "identifier" } });
        let doc = policy(json!({ "id": "c", "forbid": { "ast": q }, "violation": { "code": "X", "message": "m", "anchor": "witness.span" } }));
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        let crate::ir::InputKind::Pattern { query } = &p.inputs[0].kind else { panic!() };
        assert_eq!(query["rule"], json!({ "all": [{ "filePath": { "regex": "^impl/", "notRegex": "tests/" } }, { "kind": "identifier" }] }));
        assert!(query.get("file").is_none());
        let mut q2 = q.clone();
        q2.as_object_mut().unwrap().remove("syntax");
        let doc = policy(json!({ "id": "c", "forbid": { "ast": q2 }, "violation": { "code": "X", "message": "m", "anchor": "witness.span" } }));
        let p = compile(&doc, &Default::default(), &mut Accept).unwrap();
        let crate::ir::InputKind::Pattern { query } = &p.inputs[0].kind else { panic!() };
        assert!(query.get("file").is_some());
    }
}
