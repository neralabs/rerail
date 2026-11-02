//! One path per edit: per-content exports change → Y1 `apply` (the maintained reference graph)
//! → the step's edge delta as Y2's `Refs` presence changes → the symbol table's step (`sym`) →
//! Y2's scoped update (dead code) → findings delta. No dataflow anywhere in the path. As copied
//! from facts-b, less `exported-families` and the three small plans (M2), which are not ported.
//!
//! Every component is layered the same way: a frozen base, a frozen overlay per cluster of
//! worktrees (Spike B's commit-date ranges, plurality membership), and per worktree the rows that
//! differ. A layer is built by the same step as an edit: the base from nothing (the rules from
//! scratch over it), an overlay and a worktree as the edit that turns the layer below into it.
use super::sym::{self, PathChange, SymShared, SymState};
use crate::code_rule_dsl::facts::deadcode::digest::SetDigest;
use crate::code_rule_dsl::facts::lmap::LMap;
use crate::code_rule_dsl::facts::y2::dc::{self, Rule};
use crate::code_rule_dsl::facts::y2::facts::{sort_changes, sym as ysym, Change, Facts, Row, Tab, Val};
use crate::code_rule_dsl::facts::ygraph::path;
use crate::code_rule_dsl::facts::ygraph::world::{self as y1, Base, Below, Delta, PathRec};
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Instant;

/// Where a (path, content)'s per-content inputs come from when no step has read them yet: its
/// exports (`facts::xinputs::exports`) and whether the producer failed on the content; None when
/// the content is unknown (the steps then refuse it, as a member without exports).
pub trait Source {
    fn exports(&mut self, path: &str, content: &str) -> Option<(bool, crate::code_rule_dsl::facts::xinputs::exports::Exports)>;
}

/// What every layer and worktree shares.
pub struct Shared {
    pub y1: y1::Shared,
    pub sym: SymShared,
    /// Y1's path symbols in Y2's interner (the two interners meet only here).
    pub y2_path: FxHashMap<u32, u32>,
    /// The inputs' source; None when every input was loaded up front (`summary::load`,
    /// `input::load`).
    pub source: Option<Box<dyn Source>>,
    /// Wall time spent reading inputs from the source, since the caller last took it.
    pub inputs_ms: f64,
}

impl Shared {
    fn y2p(&mut self, p: u32) -> u32 {
        let Shared { y1, y2_path, .. } = self;
        *y2_path.entry(p).or_insert_with(|| ysym(y1.it.str(p)))
    }

    /// Reads from the source the inputs of every (path, content) of `changes` no step has read.
    fn ensure(&mut self, changes: &[&PathEdit]) {
        let Shared { y1, sym, source, inputs_ms, .. } = self;
        let Some(source) = source.as_mut() else { return };
        let t = Instant::now();
        for (p, o, n) in changes {
            for c in [o, n].into_iter().flatten() {
                let (ps, cs) = (y1.it.intern(p), y1.it.intern(c));
                if y1.sums.get(ps, cs).is_some() {
                    continue;
                }
                if let Some((failure, ex)) = source.exports(p, c) {
                    crate::code_rule_dsl::facts::ygraph::summary::insert(&mut y1.it, &mut y1.sums, p, c, failure, &ex);
                    super::input::insert(&mut sym.blocks, p, c, &ex);
                }
            }
        }
        *inputs_ms += t.elapsed().as_secs_f64() * 1e3;
    }
}

/// One stack's mutable top: a worktree, or a layer under construction.
pub struct Top {
    pub y1: Delta,
    pub sym: SymState,
    pub facts: Facts,
    /// Dead code's roots (imported-module counts); its reachability is `facts::deadcode`'s.
    pub rule: Option<Rule>,
    /// Reverse edge index: target def → source defs (Y1's DefIds).
    pub rev: LMap<u32, Vec<u32>>,
    /// path → content (Y2's interner)
    pub members: LMap<u32, u32>,
    /// Digests of the stack's `Refs` rows (the reference graph's derivation output) and of its
    /// symbol-table rows, maintained by each step's presence changes.
    pub refs_digest: SetDigest,
    pub sym_digest: SetDigest,
}

/// A frozen layer of every component.
pub struct Layer {
    pub y1: Base,
    pub top: Top,
}

impl Top {
    /// A worktree (or overlay) over the frozen layer `l`.
    pub fn over(l: &Layer, below: &Below) -> Top {
        Top {
            y1: Delta::at(below),
            sym: l.top.sym.view(),
            facts: l.top.facts.view(),
            rule: l.top.rule.as_ref().map(Rule::view),
            rev: l.top.rev.view(),
            members: l.top.members.view(),
            refs_digest: l.top.refs_digest,
            sym_digest: l.top.sym_digest,
        }
    }

    pub fn freeze(mut self, overlay: bool) -> Layer {
        self.sym.freeze();
        self.facts.freeze();
        if let Some(r) = self.rule.as_mut() {
            r.freeze();
        }
        self.rev.freeze();
        self.members.freeze();
        let y1 = std::mem::take(&mut self.y1).freeze(overlay);
        Layer { y1, top: self }
    }

    /// Bytes by component: the top (this worktree's difference), or with `layer` the top-most
    /// frozen layer.
    pub fn bytes(&self, layer: bool) -> serde_json::Value {
        let (ft, fl) = self.facts.bytes();
        let (dt, dl) = self.rule.as_ref().map_or((0, 0), Rule::bytes);
        let pick = |t: usize, l: usize| if layer { l } else { t };
        serde_json::json!({
            "sym": pick(self.sym.top_bytes(), self.sym.layer_bytes()),
            "y2_inputs": pick(ft, fl),
            "dc": pick(dt, dl),
            "rev": pick(self.rev.top_bytes(), self.rev.layer_bytes(0)),
            "members": pick(self.members.top_bytes(), self.members.layer_bytes(0)),
        })
    }
}

/// One step's work.
#[derive(Default, Debug, serde::Serialize)]
pub struct StepOut {
    pub paths: usize,
    pub y1_ms: f64,
    pub y1: y1::Step,
    pub refs_ms: f64,
    pub refs_changes: usize,
    pub sym: sym::Work,
    pub sym_changes: usize,
    pub y2_ms: f64,
    pub ms: f64,
    /// Dead code's scope (when the top holds the rule's state): what the product graph's rows
    /// and the findings re-evaluate.
    #[serde(skip)]
    pub scope: Option<dc::Scope>,
}

pub const DEAD_CODE: &str = "unreachable-internal-symbols";

/// A position row of `Refs` (source def's start → target def's start).
fn refs_row(p: u32, s: u32, tp: u32, ts: u32) -> Row {
    Box::new([Val::S(p), Val::Int(s as i64), Val::Int(0), Val::S(tp), Val::Int(ts as i64)])
}

/// The start of def `def` in record `r` of its path.
fn start_of(sh: &y1::Shared, r: Option<&PathRec>, def: u32) -> Option<u32> {
    let (_, name, occ) = sh.defs.defs[def as usize];
    let r = r?;
    path::find_def(&r.defs, name, occ).map(|i| r.defs[i].start)
}

/// The step's edges as `Refs` presence changes. Positions move when a path is re-read, so the
/// rows that change are those of the pairs out of every path whose edges were recomputed (before
/// and after) and of the pairs into a re-read path from any other source (through the reverse
/// index: their pair is unchanged, their target's start may not be).
fn refs_delta(sh: &mut Shared, below: &Below, d: &Delta, rev: &LMap<u32, Vec<u32>>, st: &y1::Step) -> Vec<Change> {
    let od: FxHashSet<u32> = st.out_pairs.iter().map(|x| x.0).collect();
    let rec_new = |p: u32| d.rec(below, p);
    let rec_old = |p: u32| match st.old_recs.get(&p) {
        Some(r) => r.as_ref(),
        None => d.rec(below, p),
    };
    let mut old: FxHashSet<(u32, u32, u32, u32)> = FxHashSet::default();
    let mut new: FxHashSet<(u32, u32, u32, u32)> = FxHashSet::default();
    fn pos<'r>(sh: &y1::Shared, rec: &dyn Fn(u32) -> Option<&'r PathRec>, s: u32, t: u32) -> (u32, u32, u32, u32) {
        let (sp, tp) = (sh.defs.defs[s as usize].0, sh.defs.defs[t as usize].0);
        let ss = start_of(sh, rec(sp), s).unwrap_or_else(|| panic!("refs: source def {s} has no position"));
        let ts = start_of(sh, rec(tp), t).unwrap_or_else(|| panic!("refs: target def {t} has no position"));
        (sp, ss, tp, ts)
    }
    for (_, before, after) in &st.out_pairs {
        for &(s, t) in before {
            old.insert(pos(&sh.y1, &rec_old, s, t));
        }
        for &(s, t) in after {
            new.insert(pos(&sh.y1, &rec_new, s, t));
        }
    }
    for (&p, orec) in &st.old_recs {
        let mut targets: FxHashSet<u32> = FxHashSet::default();
        for r in [orec.as_ref(), rec_new(p)].into_iter().flatten() {
            for (x, occ) in r.defs.iter().zip(path::occurrences(&r.defs)) {
                if let Some(id) = sh.y1.defs.get(p, x.name, occ) {
                    targets.insert(id);
                }
            }
        }
        for t in targets {
            for &s in rev.at(&t) {
                if od.contains(&sh.y1.defs.defs[s as usize].0) {
                    continue;
                }
                old.insert(pos(&sh.y1, &rec_old, s, t));
                new.insert(pos(&sh.y1, &rec_new, s, t));
            }
        }
    }
    let mut out = Vec::new();
    for (x, added) in old.difference(&new).map(|x| (x, false)).chain(new.difference(&old).map(|x| (x, true))) {
        let (p, s, tp, ts) = *x;
        let (p, tp) = (sh.y2p(p), sh.y2p(tp));
        out.push(Change { tab: Tab::Refs, row: refs_row(p, s, tp, ts), added });
    }
    out
}

/// Every edge of a stack as (source def, target def) with `Refs` rows (layers built from nothing).
pub fn all_edges(sh: &mut Shared, below: &Below, d: &Delta) -> (Vec<(u32, u32)>, Vec<Change>) {
    let mut pairs = Vec::new();
    let mut rows = Vec::new();
    for (p, r) in d.members(below) {
        if !r.linked {
            continue;
        }
        let ids: FxHashMap<u32, (u32, u32)> = r.defs.iter().zip(path::occurrences(&r.defs)).map(|(x, occ)| (x.idx, (sh.y1.defs.id(p, x.name, occ), x.start))).collect();
        for &(idx, t) in d.out(below, p) {
            let Some(&(s, ss)) = ids.get(&idx) else { continue };
            let tp = sh.y1.defs.defs[t as usize].0;
            let ts = start_of(&sh.y1, d.rec(below, tp), t).expect("edge target def is in its path");
            pairs.push((s, t));
            rows.push((p, ss, tp, ts));
        }
    }
    rows.sort_unstable();
    rows.dedup();
    let rows = rows.into_iter().map(|(p, s, tp, ts)| {
        let (p, tp) = (sh.y2p(p), sh.y2p(tp));
        Change { tab: Tab::Refs, row: refs_row(p, s, tp, ts), added: true }
    }).collect();
    (pairs, rows)
}

/// A path change as the step reads it: (path, old content, new content), contents as text.
pub type PathEdit = (String, Option<String>, Option<String>);

/// Y1's half of a step (the reference graph's derivation): the edge delta as `Refs` presence
/// changes, Y1's step, and the reverse index updated.
pub struct Xref {
    pub refs: Vec<Change>,
    pub y1: y1::Step,
    pub refs_ms: f64,
}

/// Y1 over `changes` (which change content): Y1's `apply`, the edge delta as position rows, the
/// reverse index.
pub fn xref_step(sh: &mut Shared, below: &Below, top: &mut Top, changes: &[&PathEdit]) -> Xref {
    sh.ensure(changes);
    let y1c: Vec<y1::Change> = changes.iter().map(|(p, o, n)| y1::Change { path: sh.y1.it.intern(p), old: o.as_ref().map(|x| sh.y1.it.intern(x)), new: n.as_ref().map(|x| sh.y1.it.intern(x)) }).collect();
    let st = y1::apply(&mut sh.y1, below, &mut top.y1, &y1c);
    let t = Instant::now();
    let refs = refs_delta(sh, below, &top.y1, &top.rev, &st);
    for &(s, t) in &st.edges_removed {
        top.rev.pull(t, &s);
    }
    for &(s, t) in &st.edges_added {
        top.rev.push(t, s);
    }
    top.refs_digest.changes(&refs, &mut Vec::new());
    Xref { refs, y1: st, refs_ms: t.elapsed().as_secs_f64() * 1e3 }
}

/// The symbol table over `changes` (the symbol table's derivation): the membership, then the
/// table's presence changes.
pub fn symbols_step(sh: &mut Shared, top: &mut Top, changes: &[&PathEdit]) -> (Vec<Change>, sym::Work) {
    sh.ensure(changes);
    let pc: Vec<PathChange> = changes.iter().map(|(p, o, n)| PathChange { path: ysym(p), old: o.as_deref().map(ysym), new: n.as_deref().map(ysym) }).collect();
    for c in &pc {
        match c.new {
            Some(n) => top.members.insert(c.path, n),
            None => top.members.remove(&c.path),
        };
    }
    let (out, work) = top.sym.apply(&mut sh.sym, &pc);
    top.sym_digest.changes(&out, &mut Vec::new());
    (out, work)
}

/// Y2's inputs: the presence changes of both halves applied, scoped under the old state when
/// the top holds the rule's state (then the rule's roots follow the imports).
pub fn facts_step(top: &mut Top, mut ch: Vec<Change>) -> Option<dc::Scope> {
    sort_changes(&mut ch);
    match top.rule.as_mut() {
        Some(rule) => {
            let scope = dc::scope(&top.facts, &ch);
            top.facts.apply(&ch);
            rule.apply(&scope);
            Some(scope)
        }
        None => {
            top.facts.apply(&ch);
            None
        }
    }
}

/// One step over `below` for `top`: Y1, the edge delta, the symbol table, Y2's inputs (scoped
/// when the top holds the rule's state). `changes` are (path, old content, new content) as text.
pub fn step(sh: &mut Shared, below: &Below, top: &mut Top, changes: &[PathEdit]) -> StepOut {
    let t0 = Instant::now();
    let changes: Vec<&PathEdit> = changes.iter().filter(|c| c.1 != c.2).collect();
    let mut out = StepOut { paths: changes.len(), ..Default::default() };
    let x = xref_step(sh, below, top, &changes);
    out.y1_ms = x.y1.ms;
    out.refs_changes = x.refs.len();
    out.refs_ms = x.refs_ms;
    let (sc, sw) = symbols_step(sh, top, &changes);
    out.sym_changes = sc.len();
    out.sym = sw;
    let mut ch = x.refs;
    ch.extend(sc);
    let t = Instant::now();
    out.scope = facts_step(top, ch);
    out.y2_ms = t.elapsed().as_secs_f64() * 1e3;
    out.y1 = x.y1;
    out.ms = t0.elapsed().as_secs_f64() * 1e3;
    out
}

/// The base layer: every member added to nothing (the same step as an edit: from nothing its
/// changes are every row), then the rule's roots from scratch over it.
pub fn base_layer(sh: &mut Shared, members: &[(String, String)]) -> (Layer, serde_json::Value) {
    let t = Instant::now();
    let mut top = Top { y1: Delta::default(), sym: SymState::default(), facts: Facts::default(), rule: None, rev: LMap::default(), members: LMap::default(), refs_digest: SetDigest::default(), sym_digest: SetDigest::default() };
    let changes: Vec<(String, Option<String>, Option<String>)> = members.iter().map(|(p, h)| (p.clone(), None, Some(h.clone()))).collect();
    let st = step(sh, &Below::default(), &mut top, &changes);
    top.rule = Some(Rule::scratch(&top.facts));
    let info = serde_json::json!({"members": members.len(), "ms": t.elapsed().as_secs_f64() * 1e3, "y1_ms": st.y1_ms, "refs_ms": st.refs_ms, "edges": st.refs_changes, "sym_ms": st.sym.ms, "y2_inputs_ms": st.y2_ms, "sym": st.sym, "y1": st.y1});
    (top.freeze(false), info)
}

/// An overlay (or a worktree) over `l`: the step from `l`'s membership to `members`.
pub fn over(sh: &mut Shared, below: &Below, l: &Layer, members: &[(String, String)]) -> (Top, StepOut) {
    let mut top = Top::over(l, below);
    let changes = diff(&l.top.members, members);
    let st = step(sh, below, &mut top, &changes);
    (top, st)
}

/// The changes that turn membership `from` into `to`.
pub fn diff(from: &LMap<u32, u32>, to: &[(String, String)]) -> Vec<(String, Option<String>, Option<String>)> {
    let to_m: FxHashMap<u32, u32> = to.iter().map(|(p, h)| (ysym(p), ysym(h))).collect();
    let s = crate::code_rule_dsl::facts::y2::facts::string;
    let mut out: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for (p, c) in &to_m {
        let old = from.get(p).copied();
        if old != Some(*c) {
            out.push((s(*p), old.map(s), Some(s(*c))));
        }
    }
    for (p, c) in from.iter() {
        if !to_m.contains_key(p) {
            out.push((s(*p), Some(s(*c)), None));
        }
    }
    out.sort();
    out
}

impl Layer {
    pub fn bytes(&self) -> serde_json::Value {
        let mut b = self.top.bytes(true);
        b["y1"] = serde_json::json!(self.y1.total_bytes());
        b
    }
}
