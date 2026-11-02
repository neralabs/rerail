//! Dead code (`unreachable-internal-symbols`) as cross-file derivations on `propagate`
//! (docs/final-shape/03-architecture.md §2, §14), from facts-engine's Y1 (the maintained
//! reference graph), Y3 (the symbol table, the wiring) and E3 (the maintained lattice).
//!
//! Per worktree `w` (scope `wtNN`) the observer binds each changed path (`{scope}#path:{path}`)
//! and the membership's digest (`{scope}#tree:`), and five derivations follow:
//!
//! - `{scope}#xref:` reads the tree name: Y1's step over the changed paths, its output the
//!   worktree's `Refs` rows (the reference edges by position) (`wire::xref_step`).
//! - `{scope}#symbols:` reads the tree name: the symbol table's step, its output the table's rows
//!   (`wire::symbols_step`).
//! - `{scope}#deadcode.graph:` reads both: Y2's inputs applied and scoped (`wire::facts_step`),
//!   the product graph's rows re-evaluated there (`reach::rows`); its output the CSR's edges and
//!   roots, by state.
//! - `{scope}#deadcode.reached:` reads the graph: `min_level` over the CSR, recomputed or
//!   maintained (`reach::Mode`); its output the reached states.
//! - `{scope}#deadcode.findings:` reads the reached set and Y2's inputs (xref, symbols): the
//!   findings of the definitions whose reads changed (the scope's, and those reading a toggled
//!   annotated state); its output the finding rows.
//!
//! Every output digest is a set digest maintained by the step's delta (`digest`), so a step that
//! changes no row of its output rebinds nothing and its readers never run (early cutoff). A step
//! hands its delta to its readers through this producer (`Pending`), as `Families` hands tapes
//! from the tape wave to the core wave: the derivation rows record what was read and what came
//! out; the rows themselves stay in the producers' layered state (Y3's `Top`, `reach::LWt`).
//!
//! The findings derivation reads the reached set and, directly, Y2's inputs; `propagate` has no
//! level order, so after an edit that changes both it runs twice: once with the graph (the
//! scope's definitions, under the reached set before the edit), once after the reached set (the
//! readers of the toggled states). Each run's output is the findings of the inputs bound when it
//! ran; `Work` counts the second.
pub(crate) mod digest;
pub(crate) mod reach;
#[cfg(test)]
mod replay;
#[cfg(test)]
mod tests;

use super::Graph;
use super::lattice::maint::Edit;
use super::lattice::round::{INF, Rows};
use super::observe::{path_name, tree_name};
use super::propagate::{Work, propagate};
use super::y2::dc::{self, Dc, Finding};
use super::y2::facts::{string, Change, Tab, Val};
use super::y3::sym;
use super::y3::wire::{self, PathEdit, Top};
use super::y3::world::{World, below};
use crate::content_store::{ABSENT, Binding, Derivation, DerivationInput, DerivationRecord, StoreError, StoreResult};
use digest::SetDigest;
use reach::{Base, LWt, Mode, RowCounts, Tmp, ValueCounts};
use rustc_hash::{FxHashMap, FxHashSet};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Instant;

/// The five families, in the order a wave runs them (a reader after what it reads).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Family {
    Xref,
    Symbols,
    Graph,
    Reached,
    Findings,
}

const FAMILIES: [Family; 5] = [Family::Xref, Family::Symbols, Family::Graph, Family::Reached, Family::Findings];

impl Family {
    fn tag(self) -> &'static str {
        match self {
            Family::Xref => "#xref:",
            Family::Symbols => "#symbols:",
            Family::Graph => "#deadcode.graph:",
            Family::Reached => "#deadcode.reached:",
            Family::Findings => "#deadcode.findings:",
        }
    }

    pub(crate) fn name(self, scope: &str) -> String {
        format!("{scope}{}", self.tag())
    }

    /// The producer's identity (its revision included). The reached set's is the same in both
    /// modes: they compute one value.
    fn producer(self) -> [u8; 32] {
        let tag = match self {
            Family::Xref => "rerail.xref/y1-v1",
            Family::Symbols => "rerail.symbols/y3-v1",
            Family::Graph => "rerail.deadcode.graph/v1",
            Family::Reached => "rerail.deadcode.reached/min-level-v1",
            Family::Findings => "rerail.deadcode.findings/v1",
        };
        Sha256::digest(format!("{tag}\0")).into()
    }

    /// The scope and family a derivation's output names.
    fn of(output: &str) -> Option<(&str, Family)> {
        FAMILIES.into_iter().find_map(|f| output.strip_suffix(f.tag()).map(|scope| (scope, f)))
    }
}

/// The scope a worktree's names live under.
pub(crate) fn scope_of(w: u32) -> String {
    format!("wt{w:02}")
}

/// A worktree's inputs (Y3's world, or a test's stand-in): the stack top holding Y2's facts and
/// the rule's roots, and the two upstream producers.
pub(crate) trait Inputs {
    fn top(&self, w: u32) -> &Top;
    fn top_mut(&mut self, w: u32) -> &mut Top;
    /// Y1's step over `edits` (`wire::xref_step`).
    fn xref(&mut self, w: u32, edits: &[&PathEdit]) -> wire::Xref;
    /// The symbol table's step over `edits` (`wire::symbols_step`).
    fn symbols(&mut self, w: u32, edits: &[&PathEdit]) -> (Vec<Change>, sym::Work);
    /// Every symbol-table row of `w` from scratch, for the digest check (None: not kept).
    fn symbol_rows(&self, w: u32) -> Option<Vec<(Tab, Box<[Val]>)>>;
    /// The membership (path, content) `f`'s state of `w` holds (`Xref`: Y1's, `Symbols`: the
    /// table's), from which a step's delta is rebuilt when the push's is not at hand.
    fn members(&self, w: u32, f: Family) -> Vec<(String, String)>;
}

impl Inputs for World {
    fn top(&self, w: u32) -> &Top {
        &self.wts[&w].top
    }
    fn top_mut(&mut self, w: u32) -> &mut Top {
        &mut self.wts.get_mut(&w).unwrap_or_else(|| panic!("worktree {w} unknown")).top
    }
    fn xref(&mut self, w: u32, edits: &[&PathEdit]) -> wire::Xref {
        let World { sh, layers, wts } = self;
        let wt = wts.get_mut(&w).unwrap_or_else(|| panic!("worktree {w} unknown"));
        wire::xref_step(sh, &below(layers, wt.layer), &mut wt.top, edits)
    }
    fn symbols(&mut self, w: u32, edits: &[&PathEdit]) -> (Vec<Change>, sym::Work) {
        let World { sh, wts, .. } = self;
        let wt = wts.get_mut(&w).unwrap_or_else(|| panic!("worktree {w} unknown"));
        wire::symbols_step(sh, &mut wt.top, edits)
    }
    fn symbol_rows(&self, w: u32) -> Option<Vec<(Tab, Box<[Val]>)>> {
        Some(self.wts[&w].top.sym.all_rows(&self.sh.sym))
    }
    fn members(&self, w: u32, f: Family) -> Vec<(String, String)> {
        let wt = &self.wts[&w];
        match f {
            Family::Xref => {
                let it = &self.sh.y1.it;
                wt.top.y1.members(&below(&self.layers, wt.layer)).into_iter().map(|(p, r)| (it.str(p).to_owned(), it.str(r.content).to_owned())).collect()
            }
            _ => wt.top.members.iter().map(|(p, c)| (string(*p), string(*c))).collect(),
        }
    }
}

/// What a step hands to the next family's run. The path edits are a cache: a family uses them
/// only when they turn the membership its state holds into the one bound now (`delta`).
#[derive(Default)]
struct Pending {
    xref: Vec<PathEdit>,
    symbols: Vec<PathEdit>,
    refs: Vec<Change>,
    sym: Vec<Change>,
    edit: Option<Edit>,
    findings: FxHashSet<u32>,
    toggled: Vec<u32>,
}

/// One worktree's dead-code state and output digests.
struct Wt {
    scope: String,
    reach: LWt,
    findings: FxHashMap<u32, Vec<Finding>>,
    tree: SetDigest,
    /// The membership digests Y1's and the symbol table's states hold (each `tree` after a run).
    xref_tree: SetDigest,
    sym_tree: SetDigest,
    graph: SetDigest,
    reached: SetDigest,
    found: SetDigest,
    pending: Pending,
}

/// The producers' work over one push (or creation): wall time per component, counts.
#[derive(Clone, Default, Debug, serde::Serialize)]
pub(crate) struct Step {
    /// Y1's apply.
    pub y1_ms: f64,
    /// The edge delta as `Refs` rows, the reverse index and the output digest.
    pub refs_ms: f64,
    pub refs_changes: usize,
    /// The symbol table's step and its output digest.
    pub sym_ms: f64,
    pub sym_changes: usize,
    /// Y2's scoping under the old state, then its inputs applied (with the roots' counts).
    pub scope_ms: f64,
    pub apply_ms: f64,
    pub rows: RowCounts,
    pub values: ValueCounts,
    pub findings_ms: f64,
    pub findings_evaluated: usize,
    pub findings_changed: usize,
    /// The graph's, reached set's and findings' output digests.
    pub digest_ms: f64,
    /// Runs per family (xref, symbols, graph, reached, findings).
    pub runs: [u32; 5],
    /// Xref or symbols runs whose delta was rebuilt from the store's bindings (no push's edits).
    pub rebuilt: u32,
}

/// Dead code over a set of worktrees, behind `propagate`.
pub(crate) struct DeadCode<I: Inputs> {
    pub inputs: I,
    pub base: Base,
    pub mode: Mode,
    wts: BTreeMap<u32, Wt>,
    tmp: Tmp,
    /// The base's graph and reached set, from which a worktree's digests start.
    base_graph: SetDigest,
    base_reached: SetDigest,
    buf: Vec<u8>,
    /// Accumulated since the caller last took it.
    pub step: Step,
}

/// A content named by its blake3 hex (the exports' and events' form) as a binding's digest;
/// any other text by its SHA-256.
fn content_digest(c: Option<&str>) -> [u8; 32] {
    let Some(c) = c else { return ABSENT };
    let b = c.as_bytes();
    if b.len() == 64 && b.iter().all(u8::is_ascii_hexdigit) {
        let hex = |x: u8| (x as char).to_digit(16).unwrap() as u8;
        std::array::from_fn(|i| hex(b[2 * i]) << 4 | hex(b[2 * i + 1]))
    } else {
        Sha256::digest(b).into()
    }
}

fn graph_digest(d: &mut SetDigest, b: &Base, w: &LWt, e: &Edit, buf: &mut Vec<u8>) {
    let s = |v: u32| w.ext.st_of(b, v);
    for &(x, y) in &e.del {
        digest::edge_bytes(s(x), s(y), buf);
        d.remove(buf);
    }
    for &(x, y) in &e.ins {
        digest::edge_bytes(s(x), s(y), buf);
        d.add(buf);
    }
    for &v in &e.seeds_del {
        digest::seed_bytes(s(v), buf);
        d.remove(buf);
    }
    for &v in &e.seeds_ins {
        digest::seed_bytes(s(v), buf);
        d.add(buf);
    }
}

fn reached_digest(d: &mut SetDigest, b: &Base, w: &LWt, toggled: &[u32], buf: &mut Vec<u8>) {
    let lv = w.lvl.as_slice();
    for &v in toggled {
        digest::state_bytes(w.ext.st_of(b, v), buf);
        d.toggle(buf, lv[v as usize] != INF);
    }
}

fn findings_digest(d: &mut SetDigest, old: &[Finding], new: &[Finding], buf: &mut Vec<u8>) {
    for f in old {
        digest::finding_bytes(f, buf);
        d.remove(buf);
    }
    for f in new {
        digest::finding_bytes(f, buf);
        d.add(buf);
    }
}

impl<I: Inputs> DeadCode<I> {
    /// Over `base` (built from worktree 1's facts, E3's `--base w1`).
    pub(crate) fn new(inputs: I, base: Base, mode: Mode) -> Self {
        let mut buf = Vec::new();
        let (mut base_graph, mut base_reached) = (SetDigest::default(), SetDigest::default());
        let fwd = base.fwd();
        for v in 0..base.nb as u32 {
            let s = base.st[v as usize];
            for &t in fwd.row(v) {
                digest::edge_bytes(s, base.st[t as usize], &mut buf);
                base_graph.add(&buf);
            }
            if base.lvl[v as usize] != INF {
                digest::state_bytes(s, &mut buf);
                base_reached.add(&buf);
            }
        }
        for v in base.seeds() {
            digest::seed_bytes(base.st[v as usize], &mut buf);
            base_graph.add(&buf);
        }
        DeadCode { inputs, base, mode, wts: BTreeMap::new(), tmp: Tmp::default(), base_graph, base_reached, buf, step: Step::default() }
    }

    pub(crate) fn worktrees(&self) -> impl Iterator<Item = u32> + '_ {
        self.wts.keys().copied()
    }

    pub(crate) fn reach(&self, w: u32) -> &LWt {
        &self.wts[&w].reach
    }

    /// Worktree `w` (its stack top already in the inputs): its reached set from the base, its
    /// findings from scratch, its names bound and its five derivations recorded.
    pub(crate) fn create(&mut self, graph: &(impl Graph + ?Sized), w: u32) -> StoreResult<()> {
        let scope = scope_of(w);
        let DeadCode { inputs, base, mode, tmp, buf, step, .. } = self;
        let top = inputs.top(w);
        let mut toggled = Vec::new();
        let (reach, e, rows, values) = reach::create(base, &top.facts, *mode, tmp, &mut toggled);
        let t = Instant::now();
        let mut g = self.base_graph;
        graph_digest(&mut g, base, &reach, &e, buf);
        let mut r = self.base_reached;
        reached_digest(&mut r, base, &reach, &toggled, buf);
        step.digest_ms += t.elapsed().as_secs_f64() * 1e3;
        step.rows = rows;
        step.values = values;
        let tf = Instant::now();
        let rule = top.rule.as_ref().expect("a worktree holds the rule");
        let ann = |x: u32| reach.reached(base, dc::st(x, dc::ANN));
        let mut findings = FxHashMap::default();
        let mut found = SetDigest::default();
        for (&id, d) in top.facts.defs.iter() {
            let v = rule.finding(&top.facts, d, &ann);
            if !v.is_empty() {
                findings_digest(&mut found, &[], &v, buf);
                findings.insert(id, v);
            }
        }
        step.findings_evaluated += top.facts.defs.len();
        step.findings_ms += tf.elapsed().as_secs_f64() * 1e3;
        let mut tree = SetDigest::default();
        let mut bindings = Vec::new();
        for (p, c) in top.members.iter() {
            let (p, c) = (string(*p), string(*c));
            digest::member_bytes(&p, &c, buf);
            tree.add(buf);
            bindings.push(Binding { name: path_name(&scope, &p), digest: content_digest(Some(&c)) });
        }
        bindings.push(Binding { name: tree_name(&scope, ""), digest: tree.bytes() });
        self.wts.insert(w, Wt { scope, reach, findings, tree, xref_tree: tree, sym_tree: tree, graph: g, reached: r, found, pending: Pending::default() });
        let records: Vec<DerivationRecord> = FAMILIES.into_iter().map(|f| self.record(w, f, 0)).collect();
        graph.begin_push()?;
        graph.bind_names(&bindings)?;
        graph.record_derivations(&records)?;
        graph.end_push()
    }

    /// One event in worktree `w`: the observer rebinds the changed paths and the membership's
    /// digest, then `propagate` runs every derivation that read a rebound name, wave by wave.
    pub(crate) fn push(&mut self, graph: &(impl Graph + ?Sized), w: u32, edits: &[PathEdit], max_waves: u32) -> StoreResult<Work> {
        let edits: Vec<PathEdit> = edits.iter().filter(|c| c.1 != c.2).cloned().collect();
        if edits.is_empty() {
            return Ok(Work::default());
        }
        graph.begin_push()?;
        let changed = self.observe(graph, w, &edits)?;
        let wt = self.wts.get_mut(&w).expect("observed");
        wt.pending.xref.extend(edits.iter().cloned());
        wt.pending.symbols.extend(edits);
        let work = propagate(graph, changed, max_waves, |frontier| self.recompute(graph, frontier))?;
        graph.end_push()?;
        Ok(work)
    }

    /// The observer's half of a push: `edits` (which change content) bound as `w`'s path names
    /// and membership digest; the names whose digest changed. The producers are not told: a push
    /// hands them the edits as a cache, and a recovery finds the delta from these bindings.
    pub(crate) fn observe(&mut self, graph: &(impl Graph + ?Sized), w: u32, edits: &[PathEdit]) -> StoreResult<Vec<String>> {
        let wt = self.wts.get_mut(&w).ok_or_else(|| StoreError::Invalid(format!("worktree {w} unknown")))?;
        let mut bindings = Vec::with_capacity(edits.len() + 1);
        for (p, o, n) in edits {
            if let Some(o) = o {
                digest::member_bytes(p, o, &mut self.buf);
                wt.tree.remove(&self.buf);
            }
            if let Some(n) = n {
                digest::member_bytes(p, n, &mut self.buf);
                wt.tree.add(&self.buf);
            }
            bindings.push(Binding { name: path_name(&wt.scope, p), digest: content_digest(n.as_deref()) });
        }
        bindings.push(Binding { name: tree_name(&wt.scope, ""), digest: wt.tree.bytes() });
        graph.bind_names(&bindings)
    }

    /// The edits that turn the membership family `f`'s state of `w` holds (digest `applied`) into
    /// the one bound now (O39): none when they agree; the push's edits when they turn the one into
    /// the other; else the difference between `f`'s membership and the path names bound under
    /// the scope (read from SQLite: after a crash, before any push). With the new digest.
    fn delta(&mut self, graph: &(impl Graph + ?Sized), w: u32, f: Family, cached: Vec<PathEdit>) -> StoreResult<(Vec<PathEdit>, SetDigest)> {
        let wt = &self.wts[&w];
        let applied = if f == Family::Xref { wt.xref_tree } else { wt.sym_tree };
        let bound = graph.binding(&tree_name(&wt.scope, ""))?.unwrap_or(ABSENT);
        if applied.bytes() == bound {
            return Ok((Vec::new(), applied));
        }
        let mut d = applied;
        for (p, o, n) in &cached {
            if let Some(o) = o {
                digest::member_bytes(p, o, &mut self.buf);
                d.remove(&self.buf);
            }
            if let Some(n) = n {
                digest::member_bytes(p, n, &mut self.buf);
                d.add(&self.buf);
            }
        }
        if d.bytes() == bound {
            return Ok((cached, d));
        }
        self.step.rebuilt += 1;
        let prefix = path_name(&wt.scope, "");
        let mut stored: BTreeMap<String, String> = BTreeMap::new();
        for (name, digest) in graph.store().bindings_with_prefix(&prefix)? {
            if digest != ABSENT {
                stored.insert(name[prefix.len()..].to_owned(), digest.iter().map(|b| format!("{b:02x}")).collect());
            }
        }
        let held: BTreeMap<String, String> = self.inputs.members(w, f).into_iter().collect();
        let mut edits = Vec::new();
        for (p, c) in &stored {
            if held.get(p) != Some(c) {
                edits.push((p.clone(), held.get(p).cloned(), Some(c.clone())));
            }
        }
        for (p, c) in &held {
            if !stored.contains_key(p) {
                edits.push((p.clone(), Some(c.clone()), None));
            }
        }
        let mut d = SetDigest::default();
        for (p, c) in &stored {
            digest::member_bytes(p, c, &mut self.buf);
            d.add(&self.buf);
        }
        Ok((edits, d))
    }

    /// One wave's frontier, each derivation re-run by its family, readers after what they read.
    pub(crate) fn recompute(&mut self, graph: &(impl Graph + ?Sized), frontier: &[Derivation]) -> StoreResult<Vec<DerivationRecord>> {
        let mut jobs = Vec::with_capacity(frontier.len());
        for d in frontier {
            let (scope, f) = Family::of(&d.output).ok_or_else(|| StoreError::Invalid(format!("not a dead-code derivation: {}", d.output)))?;
            let w: u32 = scope.strip_prefix("wt").and_then(|x| x.parse().ok()).ok_or_else(|| StoreError::Invalid(format!("not a worktree scope: {scope}")))?;
            if !self.wts.contains_key(&w) {
                return Err(StoreError::Invalid(format!("worktree {w} unknown")));
            }
            jobs.push((w, f));
        }
        jobs.sort_unstable();
        let mut records = Vec::with_capacity(jobs.len());
        for (w, f) in jobs {
            let t = Instant::now();
            self.run(graph, w, f)?;
            records.push(self.record(w, f, t.elapsed().as_micros() as i64));
        }
        Ok(records)
    }

    fn run(&mut self, graph: &(impl Graph + ?Sized), w: u32, f: Family) -> StoreResult<()> {
        let cached = match f {
            Family::Xref => Some(std::mem::take(&mut self.wts.get_mut(&w).unwrap().pending.xref)),
            Family::Symbols => Some(std::mem::take(&mut self.wts.get_mut(&w).unwrap().pending.symbols)),
            _ => None,
        };
        let delta = cached.map(|c| self.delta(graph, w, f, c)).transpose()?;
        let DeadCode { inputs, base, mode, wts, tmp, buf, step, .. } = self;
        let wt = wts.get_mut(&w).unwrap();
        step.runs[f as usize] += 1;
        match f {
            Family::Xref => {
                let (edits, tree) = delta.unwrap();
                let refs: Vec<&PathEdit> = edits.iter().collect();
                let x = inputs.xref(w, &refs);
                wt.xref_tree = tree;
                step.y1_ms += x.y1.ms;
                step.refs_ms += x.refs_ms;
                step.refs_changes += x.refs.len();
                wt.pending.refs.extend(x.refs);
            }
            Family::Symbols => {
                let (edits, tree) = delta.unwrap();
                let refs: Vec<&PathEdit> = edits.iter().collect();
                let t = Instant::now();
                let (sc, _) = inputs.symbols(w, &refs);
                wt.sym_tree = tree;
                step.sym_ms += t.elapsed().as_secs_f64() * 1e3;
                step.sym_changes += sc.len();
                wt.pending.sym.extend(sc);
            }
            Family::Graph => {
                let mut ch = std::mem::take(&mut wt.pending.refs);
                ch.append(&mut wt.pending.sym);
                let t = Instant::now();
                let top = inputs.top_mut(w);
                let scope = wire::facts_step(top, ch).ok_or_else(|| StoreError::Invalid(format!("worktree {w} holds no dead-code rule")))?;
                step.scope_ms += scope.ms;
                step.apply_ms += t.elapsed().as_secs_f64() * 1e3 - scope.ms;
                let top = inputs.top(w);
                let mut rows = RowCounts::default();
                let e = reach::rows(base, &mut wt.reach, &top.facts, top.rule.as_ref().unwrap(), &scope, tmp, &mut rows);
                step.rows = rows;
                wt.pending.findings.extend(scope.findings);
                if !(e.del.is_empty() && e.ins.is_empty() && e.seeds_del.is_empty() && e.seeds_ins.is_empty()) {
                    let t = Instant::now();
                    graph_digest(&mut wt.graph, base, &wt.reach, &e, buf);
                    step.digest_ms += t.elapsed().as_secs_f64() * 1e3;
                    if wt.pending.edit.replace(e).is_some() {
                        return Err(StoreError::Invalid(format!("worktree {w}: the graph changed twice before its reached set ran")));
                    }
                }
            }
            Family::Reached => {
                let e = wt.pending.edit.take().unwrap_or_default();
                let mut toggled = Vec::new();
                step.values = reach::values(base, &mut wt.reach, &e, *mode, tmp, &mut toggled);
                let t = Instant::now();
                reached_digest(&mut wt.reached, base, &wt.reach, &toggled, buf);
                step.digest_ms += t.elapsed().as_secs_f64() * 1e3;
                wt.pending.toggled.extend(toggled);
            }
            Family::Findings => {
                let t = Instant::now();
                let top = inputs.top(w);
                let f = &top.facts;
                let mut ids = std::mem::take(&mut wt.pending.findings);
                for v in std::mem::take(&mut wt.pending.toggled) {
                    let s = wt.reach.ext.st_of(base, v);
                    if dc::layer(s) == dc::ANN {
                        dc::toggled_readers(f, dc::node(s), &mut ids);
                    }
                }
                let rule = top.rule.as_ref().unwrap();
                let Wt { reach, findings, found, .. } = wt;
                let ann = |x: u32| reach.reached(base, dc::st(x, dc::ANN));
                step.findings_evaluated += ids.len();
                for id in ids {
                    let new = f.defs.get(&id).map(|d| rule.finding(f, d, &ann)).unwrap_or_default();
                    let old = findings.remove(&id).unwrap_or_default();
                    if old != new {
                        step.findings_changed += 1;
                        findings_digest(found, &old, &new, buf);
                    }
                    if !new.is_empty() {
                        findings.insert(id, new);
                    }
                }
                step.findings_ms += t.elapsed().as_secs_f64() * 1e3;
            }
        }
        Ok(())
    }

    /// The digest `f`'s output of `w` holds now.
    fn digest(&self, w: u32, f: Family) -> [u8; 32] {
        let wt = &self.wts[&w];
        match f {
            Family::Xref => self.inputs.top(w).refs_digest.bytes(),
            Family::Symbols => self.inputs.top(w).sym_digest.bytes(),
            Family::Graph => wt.graph.bytes(),
            Family::Reached => wt.reached.bytes(),
            Family::Findings => wt.found.bytes(),
        }
    }

    /// `f`'s derivation for `w` as it stands: what it read, what it produced.
    fn record(&self, w: u32, f: Family, cost: i64) -> DerivationRecord {
        let wt = &self.wts[&w];
        let input = |name: String, digest: [u8; 32]| DerivationInput { name: Some(name), digest };
        // What Y1 and the table read is the membership their states hold.
        let tree = |d: SetDigest| input(tree_name(&wt.scope, ""), d.bytes());
        let of = |g: Family| input(g.name(&wt.scope), self.digest(w, g));
        let inputs = match f {
            Family::Xref => vec![tree(wt.xref_tree)],
            Family::Symbols => vec![tree(wt.sym_tree)],
            Family::Graph => vec![of(Family::Xref), of(Family::Symbols)],
            Family::Reached => vec![of(Family::Graph)],
            Family::Findings => vec![of(Family::Xref), of(Family::Symbols), of(Family::Reached)],
        };
        DerivationRecord { output: f.name(&wt.scope), producer: f.producer(), inputs, digest: self.digest(w, f), status: 0, cost }
    }

    /// `w`'s findings as the dump prints them (`dc__findings`).
    pub(crate) fn finding_rows(&self, w: u32) -> Vec<Vec<Val>> {
        dc::finding_rows(self.wts[&w].findings.values())
    }

    /// `w`'s bound names against this producer's digests: every difference.
    pub(crate) fn bindings_differ(&self, graph: &(impl Graph + ?Sized), w: u32) -> StoreResult<Vec<String>> {
        let wt = &self.wts[&w];
        let mut out = Vec::new();
        let tree = tree_name(&wt.scope, "");
        if graph.binding(&tree)? != Some(wt.tree.bytes()) {
            out.push(format!("{tree}: bound digest differs"));
        }
        if wt.xref_tree != wt.tree || wt.sym_tree != wt.tree {
            out.push(format!("{tree}: Y1's or the table's membership is not the bound one"));
        }
        for f in FAMILIES {
            let name = f.name(&wt.scope);
            if graph.binding(&name)? != Some(self.digest(w, f)) {
                out.push(format!("{name}: bound digest differs"));
            }
            if !graph.derivation_valid(&name)? {
                out.push(format!("{name}: derivation invalid"));
            }
        }
        Ok(out)
    }

    /// `w` against everything from scratch: the lattice (`reach::check`: rows = Y2's `succ`, the
    /// transpose, the roots, the levels = E1's fixpoint on the maintained CSR = Y2's from-scratch
    /// reached set and levels), the findings = Y2's from scratch, and every output digest = the
    /// digest of its output computed from scratch.
    pub(crate) fn check(&self, w: u32) -> Vec<String> {
        let (b, wt, top) = (&self.base, &self.wts[&w], self.inputs.top(w));
        let f = &top.facts;
        let mut diffs = reach::check(b, &wt.reach, f);
        let y2 = Dc::scratch(f).finding_rows();
        let mine = self.finding_rows(w);
        if y2 != mine {
            let (a, z) = super::y2::diff_rows(&mine, &y2);
            diffs.push(format!("findings: {} maintained / {} Y2 from scratch; only maintained {:?}; only Y2 {:?}", mine.len(), y2.len(), &a[..a.len().min(3)], &z[..z.len().min(3)]));
        }
        let mut buf = Vec::new();
        let n = wt.reach.ext.n();
        let fr = wt.reach.fwd.rows(b.fwd());
        let s = |v: u32| wt.reach.ext.st_of(b, v);
        let (mut g, mut r) = (SetDigest::default(), SetDigest::default());
        let lv = wt.reach.lvl.as_slice();
        let sd = wt.reach.seed.as_slice();
        for v in 0..n as u32 {
            for &t in fr.row(v) {
                digest::edge_bytes(s(v), s(t), &mut buf);
                g.add(&buf);
            }
            if (v >> 6) as usize >= sd.len() {
                continue;
            }
            if sd[(v >> 6) as usize] >> (v & 63) & 1 == 1 {
                digest::seed_bytes(s(v), &mut buf);
                g.add(&buf);
            }
        }
        for v in 0..n.min(lv.len()) as u32 {
            if lv[v as usize] != INF {
                digest::state_bytes(s(v), &mut buf);
                r.add(&buf);
            }
        }
        let mut found = SetDigest::default();
        for v in wt.findings.values() {
            findings_digest(&mut found, &[], v, &mut buf);
        }
        let refs = refs_digest(f);
        let mut tree = SetDigest::default();
        for (p, c) in top.members.iter() {
            digest::member_bytes(&string(*p), &string(*c), &mut buf);
            tree.add(&buf);
        }
        for (what, got, want) in [("graph", wt.graph, g), ("reached", wt.reached, r), ("findings", wt.found, found), ("xref", top.refs_digest, refs), ("tree", wt.tree, tree)] {
            if got != want {
                diffs.push(format!("digest: {what} maintained ≠ from scratch"));
            }
        }
        if let Some(rows) = self.inputs.symbol_rows(w) {
            let mut sym = SetDigest::default();
            for (t, r) in &rows {
                digest::row_bytes(*t, r, &mut buf);
                sym.add(&buf);
            }
            if sym != top.sym_digest {
                diffs.push("digest: symbols maintained ≠ from scratch".to_owned());
            }
        }
        diffs
    }

    /// Memory of `w`'s lattice state (`reach::memory`) and its findings.
    pub(crate) fn memory(&self, w: u32) -> serde_json::Value {
        let wt = &self.wts[&w];
        let mut m = reach::memory(&self.base, &wt.reach);
        m["findings"] = serde_json::json!(wt.findings.values().map(Vec::len).sum::<usize>());
        m
    }
}

/// The digest of `f`'s `Refs` rows, from scratch (the reference edges by position).
pub(crate) fn refs_digest(f: &super::y2::facts::Facts) -> SetDigest {
    let mut buf = Vec::new();
    let mut refs = SetDigest::default();
    for ((p, st), ts) in f.refs_from.iter() {
        for (tp, tst) in ts {
            let row: [Val; 5] = [Val::S(*p), Val::Int(*st), Val::Int(0), Val::S(*tp), Val::Int(*tst)];
            digest::row_bytes(Tab::Refs, &row, &mut buf);
            refs.add(&buf);
        }
    }
    refs
}
