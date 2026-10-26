//! `unreachable-internal-symbols` (dead code) as local queries over the graph, with reachability
//! as the one maintained property.
//!
//! The rule's `reachable` and `annotation_reachable` are one reachability over a product graph
//! whose states are (node, layer): layer `NONE` is the rule's `flow(node, NULL)`, layer `d + 2`
//! is `flow(node, d)` (a dispatch search for method `d` through the class hierarchy), layer `ANN`
//! is `annotated(node)`. Every edge of that graph is a local query (`succ`): it reads the source
//! node's definition, its reference edges (`refs_from`), and keyed symbol-table rows. Roots are a
//! local predicate (`is_root`). What is *not* local is the reached set: "reachable from any root"
//! changes for an unbounded set of states when one edge goes (a chain behind it). So the reached
//! set is maintained, with a level per state (BFS depth at scratch) as the witness that every
//! reached non-root state has a reached predecessor of lower level. Insertions propagate forward.
//! A deleted edge or root removes its target only if no lower-level predecessor with an edge is
//! left (found through `pred`, a superset each candidate of which is verified against `succ`);
//! the removal cascades to higher-level successors in increasing level, then every removed state
//! with any reached predecessor is re-derived and propagated. (Plain DRed, which over-deletes
//! the whole forward closure, removed and re-derived up to 388K of a worktree's 400K states on a
//! third of the replay's edits: the module nodes make that closure most of the graph.)
//!
//! Scoping (exact; see `prepare`): an edit's presence changes name keys; `prepare` collects, under
//! the old state, every state whose `succ` or root predicate reads one of those keys (inverse
//! indexes), and every definition whose finding reads one. A state's `succ` is a deterministic
//! function of the keys it reads, so a state outside that set has the same edges before and
//! after. The same holds for a finding, plus its reads of the reached set, which `finish` adds
//! from the states whose reached status toggled.
use super::facts::{at, get, note, string, with, Change, Def, Facts, Kind, Tab, Val};
use crate::lmap::LMap;
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub type St = u64;
const NONE: u32 = 0;
const ANN: u32 = 1;
const MOD: u32 = 1 << 31;

fn st(node: u32, layer: u32) -> St {
    ((node as u64) << 32) | layer as u64
}
fn node(s: St) -> u32 {
    (s >> 32) as u32
}
fn layer(s: St) -> u32 {
    s as u32
}

#[derive(Clone)]
pub struct Rx {
    api_module: Regex,
    create: Regex,
    intent: Regex,
    grammar: Regex,
}

impl Rx {
    pub fn new() -> Rx {
        Rx {
            api_module: Regex::new(r"^(?:api|data_layer)\.").unwrap(),
            create: Regex::new(r"^_create_.*_(?:connection|validator|factory)$").unwrap(),
            intent: Regex::new(r"Intent$").unwrap(),
            grammar: Regex::new(r"Grammar$|^Sql").unwrap(),
        }
    }
}

const CTOR: [&str; 6] = ["__new__", "__init__", "__init_subclass__", "__post_init__", "__set_name__", "__class_getitem__"];

/// `x.base_name` such that `name` is one of `base_name`, `base_name || '_async'`,
/// `base_name || '_lazy'`.
fn base_cands(name: &str) -> Vec<&str> {
    let mut v = vec![name];
    if let Some(b) = name.strip_suffix("_async") {
        v.push(b);
    }
    if let Some(b) = name.strip_suffix("_lazy") {
        v.push(b);
    }
    v
}

/// `rx_replace(s, '\.?[^.]+$', '')`.
fn strip_last(s: &str) -> &str {
    let b = s.as_bytes();
    if b.is_empty() || *b.last().unwrap() == b'.' {
        return s;
    }
    let mut i = b.len();
    while i > 0 && b[i - 1] != b'.' {
        i -= 1;
    }
    if i > 0 {
        i -= 1;
    }
    &s[..i]
}

/// `dc__imported_modules` contributions of one import row: the start module and its prefixes.
fn import_chain(target: u32, module_kind: bool) -> Vec<String> {
    let t = string(target);
    let mut m = if module_kind { t.clone() } else { strip_last(&t).to_owned() };
    let mut out = Vec::new();
    loop {
        if !m.is_empty() && !out.contains(&m) {
            out.push(m.clone());
        }
        if !m.contains('.') {
            break;
        }
        let n = strip_last(&m).to_owned();
        if n == m {
            break;
        }
        m = n;
    }
    out
}

/// `min` under SQLite's TEXT order (bytes).
fn min_str(it: impl Iterator<Item = u32>) -> Option<u32> {
    let mut best: Option<(u32, String)> = None;
    for x in it {
        let s = string(x);
        if best.as_ref().is_none_or(|(_, b)| s < *b) {
            best = Some((x, s));
        }
    }
    best.map(|b| b.0)
}

/// `dc__class_names.target` for (module, name): the class an annotation name denotes.
fn class_target(f: &Facts, m: u32, n: u32) -> Option<u32> {
    let imported = min_str(at(&f.imports_by_module_name, &(m, n)).iter().filter(|i| i.symbol_kind).flat_map(|i| at(&f.resolved_by_fq, &i.target).iter().copied()).filter(|s| f.defs.get(s).is_some_and(|d| d.kind == Kind::Class)));
    imported.or_else(|| min_str(at(&f.by_module_name, &(m, n)).iter().copied().filter(|s| f.defs[s].kind == Kind::Class)))
}

fn unique_class(f: &Facts, name: u32) -> Option<u32> {
    match at(&f.classes_by_name, &name) {
        [c] => Some(*c),
        _ => None,
    }
}

fn lookup_concat(a: u32, sep: &str, b: &str) -> Option<u32> {
    with(a, |x| get(&format!("{x}{sep}{b}")))
}

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct Stats {
    pub changes: usize,
    pub dirty_states: usize,
    pub dirty_reached: usize,
    pub root_candidates: usize,
    pub edges_deleted: usize,
    pub edges_inserted: usize,
    pub roots_deleted: usize,
    pub roots_inserted: usize,
    pub over_deleted: usize,
    pub rederived: usize,
    pub propagated: usize,
    pub toggled: usize,
    pub findings_evaluated: usize,
    pub findings_changed: usize,
    pub succ_calls: usize,
    pub pred_checks: usize,
    pub prepare_ms: f64,
    pub finish_ms: f64,
    /// The findings that left (false) and came (true) in this step.
    #[serde(skip)]
    pub delta: Vec<(Finding, bool)>,
}

pub struct Prep {
    succ_old: FxHashMap<St, Vec<St>>,
    roots_old: Vec<(u32, bool)>,
    imports: Vec<(u32, bool, bool)>,
    hc: FxHashSet<u32>,
    stats: Stats,
}

pub type Finding = (u32, i64, i64, u32, &'static str);

/// The rule's maintained state. Every map is layered (`lmap`): a worktree's `Dc` starts as a
/// view of its cluster's (frozen) and holds only what differs.
pub struct Dc {
    rx: Rx,
    /// The reached states with their levels (BFS depth at scratch; an upper witness after edits).
    reached: LMap<St, u32>,
    disp_by_node: LMap<u32, Vec<u32>>,
    nodes_by_disp: LMap<u32, Vec<u32>>,
    imported: LMap<u32, u32>,
    pub findings: LMap<u32, Vec<Finding>>,
    succ_calls: usize,
}

/// Every map of `Dc`, for the layer operations.
macro_rules! dc_maps {
    ($m:ident) => {
        $m!(reached, disp_by_node, nodes_by_disp, imported, findings)
    };
}

impl Dc {
    fn empty() -> Dc {
        Dc { rx: Rx::new(), reached: LMap::default(), disp_by_node: LMap::default(), nodes_by_disp: LMap::default(), imported: LMap::default(), findings: LMap::default(), succ_calls: 0 }
    }

    /// A worktree's state over this (frozen) one.
    pub fn view(&self) -> Dc {
        macro_rules! v {
            ($($f:ident),*) => { Dc { rx: self.rx.clone(), succ_calls: 0, $($f: self.$f.view()),* } };
        }
        dc_maps!(v)
    }

    pub fn freeze(&mut self) {
        macro_rules! fr {
            ($($f:ident),*) => { $(self.$f.freeze();)* };
        }
        dc_maps!(fr);
    }

    /// Bytes of the top (this worktree's difference) and of the top-most frozen layer; entries
    /// in the top.
    pub fn bytes(&self) -> (usize, usize, usize) {
        let (mut t, mut l, mut n) = (0, 0, 0);
        macro_rules! b {
            ($($f:ident),*) => { $( t += self.$f.top_bytes(); l += self.$f.layer_bytes(0); n += self.$f.top_len(); )* };
        }
        dc_maps!(b);
        (t, l, n)
    }

    /// Per map: (bytes, entries) of the top, or with `layer` of the top-most frozen layer.
    pub fn by_map(&self, layer: bool) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        macro_rules! b {
            ($($f:ident),*) => { $(
                let v = if layer { (self.$f.layer_bytes(0), self.$f.layers().first().map_or(0, |l| l.len())) } else { (self.$f.top_bytes(), self.$f.top_len()) };
                m.insert(stringify!($f).into(), serde_json::json!(v));
            )* };
        }
        dc_maps!(b);
        serde_json::Value::Object(m)
    }

    /// Reached states.
    pub fn reached_len(&self) -> usize {
        self.reached.len()
    }

    /// From scratch: every root, reachability, every definition's finding.
    pub fn scratch(f: &Facts) -> Dc {
        let mut dc = Dc::empty();
        for is in f.imports_by_target.values() {
            for i in is {
                dc.count_import(i.target, !i.symbol_kind, 1);
            }
        }
        let mut roots: Vec<u32> = f.roots_by_symbol.keys().copied().collect();
        for r in f.roots_by_symbol.keys() {
            roots.extend(at(&f.public_by_owner, r).iter().copied());
        }
        roots.extend(dc.imported.keys().map(|m| m | MOD));
        for fq in f.dyn_fq.keys() {
            roots.extend(at(&f.resolved_by_fq, fq).iter().copied());
        }
        let work: BinaryHeap<Reverse<(u32, St)>> = roots.into_iter().map(|r| Reverse((0, st(r, NONE)))).collect();
        dc.propagate(f, work);
        let ids: Vec<u32> = f.defs.keys().copied().collect();
        for id in ids {
            let v = dc.finding(f, &f.defs[&id]);
            if !v.is_empty() {
                dc.findings.insert(id, v);
            }
        }
        dc
    }

    /// Maintained state, in u32 words: the reached set (3 per state, with its level), its two indexes, the
    /// imported-module counts (2 per module), the findings (5 per finding).
    pub fn state_words(&self) -> (usize, usize, usize, usize) {
        let idx: usize = self.disp_by_node.values().map(|v| v.len() + 1).sum::<usize>() + self.nodes_by_disp.values().map(|v| v.len() + 1).sum::<usize>();
        (self.reached.len() * 3, idx, self.imported.len() * 2, self.findings.values().map(|v| v.len() * 5).sum())
    }

    pub fn layer_counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for s in self.reached.keys() {
            match layer(*s) {
                NONE => c.0 += 1,
                ANN => c.1 += 1,
                _ => c.2 += 1,
            }
        }
        c
    }

    /// The reached states (for base + delta sizing across worktrees).
    pub fn reached_set(&self) -> FxHashSet<St> {
        self.reached.keys().copied().collect()
    }

    /// Distinct (class of the dispatched method, node) pairs among the dispatch states: the
    /// state a dispatch closure keyed by class, not by method, would hold.
    pub fn dispatch_by_class(&self, f: &Facts) -> usize {
        let mut s: FxHashSet<(u32, u32)> = FxHashSet::default();
        for &x in self.reached.keys() {
            if layer(x) >= 2 {
                let c = f.defs.get(&(layer(x) - 2)).and_then(|d| d.class_id).unwrap_or(0);
                s.insert((c, node(x)));
            }
        }
        s.len()
    }

    /// `dc__reachable` and `dc__annotation_reachable` as strings (module nodes `@module:<m>`).
    pub fn reached_strings(&self, l: u32) -> Vec<String> {
        let mut v: Vec<String> = self.reached.keys().filter(|s| layer(**s) == l).map(|s| node_string(node(*s))).collect();
        v.sort();
        v
    }

    fn count_import(&mut self, target: u32, module_kind: bool, d: i32) {
        for m in import_chain(target, module_kind) {
            let k = crate::y2::facts::sym(&m);
            let c = (self.imported.get(&k).copied().unwrap_or(0) as i32 + d) as u32;
            if c == 0 {
                self.imported.remove(&k);
            } else {
                self.imported.insert(k, c);
            }
        }
    }

    fn is_root(&self, f: &Facts, x: u32) -> bool {
        if x & MOD != 0 {
            return self.imported.contains_key(&(x & !MOD));
        }
        f.roots_by_symbol.contains_key(&x) || at(&f.public_by_symbol, &x).iter().any(|o| f.roots_by_symbol.contains_key(o)) || at(&f.resolved_by_symbol, &x).iter().any(|fq| f.dyn_fq.contains_key(fq))
    }

    fn insert(&mut self, s: St, level: u32) -> bool {
        if self.reached.contains_key(&s) {
            return false;
        }
        self.reached.insert(s, level);
        let l = layer(s);
        if l >= 2 {
            self.disp_by_node.push(node(s), l);
            self.nodes_by_disp.push(l, node(s));
        }
        true
    }

    fn remove(&mut self, s: St) {
        assert!(self.reached.remove(&s).is_some());
        let l = layer(s);
        if l >= 2 {
            self.disp_by_node.pull(node(s), &l);
            self.nodes_by_disp.pull(l, &node(s));
        }
    }

    /// Forward from `work` (state, level), lowest level first; the states it adds.
    fn propagate(&mut self, f: &Facts, mut work: BinaryHeap<Reverse<(u32, St)>>) -> Vec<St> {
        let mut added = Vec::new();
        let mut buf = Vec::new();
        while let Some(Reverse((l, s))) = work.pop() {
            if !self.insert(s, l) {
                continue;
            }
            added.push(s);
            buf.clear();
            self.succ(f, s, &mut buf);
            for &t in &buf {
                if !self.reached.contains_key(&t) {
                    work.push(Reverse((l + 1, t)));
                }
            }
        }
        added
    }

    /// `v`'s level if it has a place: 0 for a root, else 1 + the lowest level among its reached
    /// predecessors (below `below`, outside `excluded`) whose edge to `v` exists now.
    fn support(&mut self, f: &Facts, v: St, below: Option<u32>, excluded: &FxHashSet<St>, succ_new: &FxHashMap<St, Vec<St>>, pc: &mut usize) -> Option<u32> {
        if layer(v) == NONE && self.is_root(f, node(v)) {
            return Some(0);
        }
        let mut c = Vec::new();
        pred(self, f, v, &mut c);
        let mut c: Vec<(u32, St)> = c.into_iter().filter_map(|p| self.reached.get(&p).map(|&l| (l, p))).filter(|(l, p)| below.is_none_or(|b| *l < b) && !excluded.contains(p)).collect();
        c.sort_unstable();
        c.dedup();
        for (l, p) in c {
            *pc += 1;
            let edge = match succ_new.get(&p) {
                Some(x) => x.binary_search(&v).is_ok(),
                None => self.succ_sorted(f, p).binary_search(&v).is_ok(),
            };
            if edge {
                return Some(l + 1);
            }
        }
        None
    }

    /// The product graph's out-edges of `s`: local queries at `s`'s node.
    fn succ(&mut self, f: &Facts, s: St, out: &mut Vec<St>) {
        self.succ_calls += 1;
        succ(f, s, out);
    }

    fn succ_sorted(&mut self, f: &Facts, s: St) -> Vec<St> {
        let mut v = Vec::new();
        self.succ(f, s, &mut v);
        v.sort_unstable();
        v.dedup();
        v
    }

    fn dirty_node(&self, x: u32, out: &mut FxHashSet<St>) {
        out.insert(st(x, NONE));
        out.insert(st(x, ANN));
        for &l in at(&self.disp_by_node, &x) {
            out.insert(st(x, l));
        }
    }

    fn readers_class_target(&self, f: &Facts, m: u32, n: u32, out: &mut FxHashSet<St>) {
        for &s in at(&f.ann_by_name, &n) {
            if f.defs.get(&s).is_some_and(|d| d.module == m) {
                self.dirty_node(s, out);
            }
        }
    }

    /// Definitions whose finding reads class `o`'s hierarchy or base rows: `o` and its methods.
    fn hc_owner(f: &Facts, o: u32, hc: &mut FxHashSet<u32>) {
        hc.insert(o);
        hc.extend(at(&f.methods_by_class, &o).iter().copied());
    }

    /// Under the old state (nothing applied): the states whose `succ` or root predicate, and the
    /// definitions whose finding, read a key these presence changes touch.
    pub fn prepare(&mut self, f: &Facts, changes: &[Change]) -> Prep {
        let t = std::time::Instant::now();
        let calls0 = self.succ_calls;
        let mut d: FxHashSet<St> = FxHashSet::default();
        let mut rc: FxHashSet<u32> = FxHashSet::default();
        let mut hc: FxHashSet<u32> = FxHashSet::default();
        let mut imports = Vec::new();
        // Class rows changed per name: `unique_class(name)` is a derived key whose readers are
        // dirty only if its value changes.
        let mut class_names: FxHashMap<u32, (Vec<u32>, Vec<u32>)> = FxHashMap::default();
        // A definition that only moved (an edit above it in its file): every read of its row
        // other than its position sees the same values, so only position readers are dirty.
        let mut moved: FxHashMap<u32, (Option<Def>, Option<Def>)> = FxHashMap::default();
        for ch in changes.iter().filter(|c| c.tab == Tab::DeadTraits) {
            let x = Def::of(&ch.row);
            let e = moved.entry(x.id).or_default();
            if ch.added {
                e.1 = Some(x);
            } else {
                e.0 = Some(x);
            }
        }
        let shifted = |a: &Def, b: &Def| {
            let mut b = b.clone();
            (b.start, b.end, b.class_start) = (a.start, a.end, a.class_start);
            *a == b
        };
        moved.retain(|_, (a, b)| matches!((a, b), (Some(a), Some(b)) if shifted(a, b)));
        for (id, (a, b)) in &moved {
            self.dirty_node(*id, &mut d);
            for x in [a.as_ref().unwrap(), b.as_ref().unwrap()] {
                for src in at(&f.refs_to, &(x.path, x.start)) {
                    if let Some(&s) = f.by_pos.get(src) {
                        self.dirty_node(s, &mut d);
                    }
                }
            }
            hc.insert(*id);
        }
        for ch in changes {
            let r = &ch.row;
            match ch.tab {
                Tab::DeadTraits => {
                    let x = Def::of(r);
                    if moved.contains_key(&x.id) {
                        continue;
                    }
                    self.dirty_node(x.id, &mut d);
                    for &n in at(&self.nodes_by_disp, &(x.id + 2)) {
                        d.insert(st(n, x.id + 2));
                    }
                    for &(o, _) in at(&f.hier_by_base, &x.id) {
                        self.dirty_node(o, &mut d);
                        Self::hc_owner(f, o, &mut hc);
                    }
                    if let Some(c) = x.class_name {
                        for &cid in at(&f.by_module_name, &(x.module, c)) {
                            self.dirty_node(cid, &mut d);
                            for &(o, _) in at(&f.hier_by_base, &cid) {
                                self.dirty_node(o, &mut d);
                                Self::hc_owner(f, o, &mut hc);
                            }
                        }
                    }
                    for src in at(&f.refs_to, &(x.path, x.start)) {
                        if let Some(&s) = f.by_pos.get(src) {
                            self.dirty_node(s, &mut d);
                        }
                    }
                    with(x.name, |name| {
                        for b in base_cands(name) {
                            if let Some(b) = get(b) {
                                for &s in at(&f.functions_by_module_base, &(x.module, b)) {
                                    self.dirty_node(s, &mut d);
                                }
                            }
                        }
                    });
                    let id = string(x.id);
                    for (k, _) in id.match_indices('.') {
                        if let Some(p) = get(&id[..k]) {
                            for b in base_cands(&id[k + 1..]) {
                                if let Some(b) = get(b) {
                                    for &s in at(&f.methods_by_class_base, &(p, b)) {
                                        self.dirty_node(s, &mut d);
                                    }
                                }
                            }
                        }
                    }
                    let mut keys = Vec::new();
                    if x.kind == Kind::Class {
                        keys.push((x.module, x.name));
                        let e = class_names.entry(x.name).or_default();
                        if ch.added {
                            e.0.push(x.id);
                        } else {
                            e.1.push(x.id);
                        }
                    }
                    for fq in at(&f.resolved_by_symbol, &x.id) {
                        for i in at(&f.imports_by_target, fq) {
                            if i.symbol_kind {
                                keys.push((i.module, i.name));
                            }
                        }
                    }
                    for (m, n) in keys {
                        self.readers_class_target(f, m, n, &mut d);
                    }
                    self.dirty_node(x.module | MOD, &mut d);
                    for i in at(&f.imports_by_target, &x.fqname) {
                        self.dirty_node(i.module | MOD, &mut d);
                    }
                    // E11: methods of `x` if `x` is a structural protocol; methods named like
                    // `x` whose class is one (they read the list `x` is in).
                    if x.structural_protocol {
                        for &m in at(&f.methods_by_class, &x.id) {
                            self.dirty_node(m, &mut d);
                        }
                    }
                    if x.kind == Kind::Method {
                        for &m in at(&f.methods_by_name, &x.name) {
                            if f.defs[&m].class_id.and_then(|c| f.defs.get(&c)).is_some_and(|c| c.structural_protocol) {
                                self.dirty_node(m, &mut d);
                            }
                        }
                    }
                    Self::hc_owner(f, x.id, &mut hc);
                }
                Tab::Refs => {
                    if let Some(&s) = f.by_pos.get(&(r[0].s(), r[1].i())) {
                        self.dirty_node(s, &mut d);
                    }
                }
                Tab::Bases => {
                    let (o, b) = (r[0].s(), r[1].s());
                    self.dirty_node(o, &mut d);
                    self.dirty_node(b, &mut d);
                    for &m in at(&f.methods_by_class, &b) {
                        self.dirty_node(m, &mut d);
                    }
                    Self::hc_owner(f, o, &mut hc);
                }
                Tab::Hier => {
                    self.dirty_node(r[0].s(), &mut d);
                    Self::hc_owner(f, r[0].s(), &mut hc);
                }
                Tab::Public => {
                    self.dirty_node(r[0].s(), &mut d);
                    rc.insert(r[2].s());
                }
                Tab::Roots => {
                    let s = r[3].s();
                    rc.insert(s);
                    rc.extend(at(&f.public_by_owner, &s).iter().copied());
                }
                Tab::Imports => {
                    let (m, n, target, module_kind) = (r[3].s(), r[4].s(), r[5].s(), with(r[6].s(), |k| k == "module"));
                    self.dirty_node(m | MOD, &mut d);
                    self.readers_class_target(f, m, n, &mut d);
                    for c in import_chain(target, module_kind) {
                        rc.insert(crate::y2::facts::sym(&c) | MOD);
                    }
                    hc.extend(at(&f.by_fqname, &target).iter().copied());
                    imports.push((target, module_kind, ch.added));
                }
                Tab::Resolved => {
                    let (fq, s) = (r[0].s(), r[1].s());
                    for i in at(&f.imports_by_target, &fq) {
                        if i.symbol_kind {
                            self.readers_class_target(f, i.module, i.name, &mut d);
                        }
                    }
                    rc.insert(s);
                }
                Tab::Ann => self.dirty_node(r[0].s(), &mut d),
                Tab::ModuleNames => self.dirty_node(r[0].s() | MOD, &mut d),
                Tab::Dyn => {
                    if let Some(fq) = r[3].os() {
                        rc.extend(at(&f.resolved_by_fq, &fq).iter().copied());
                    }
                }
                Tab::Probe => {
                    if let Some(&x) = f.by_pos.get(&(r[0].s(), r[1].i())) {
                        hc.insert(x);
                    }
                }
                Tab::EfTraits => {}
            }
        }
        for (name, (added, removed)) in class_names {
            let old = unique_class(f, name);
            let mut now: Vec<u32> = at(&f.classes_by_name, &name).iter().copied().filter(|c| !removed.contains(c)).collect();
            now.extend(added);
            let new = if now.len() == 1 { Some(now[0]) } else { None };
            if old != new {
                for &s in at(&f.ann_by_name, &name) {
                    self.dirty_node(s, &mut d);
                }
            }
        }
        let mut stats = Stats { changes: changes.len(), dirty_states: d.len(), root_candidates: rc.len(), ..Default::default() };
        let mut succ_old = FxHashMap::default();
        for s in d {
            if self.reached.contains_key(&s) {
                let v = self.succ_sorted(f, s);
                succ_old.insert(s, v);
            }
        }
        stats.dirty_reached = succ_old.len();
        let roots_old = rc.into_iter().map(|x| (x, self.is_root(f, x))).collect();
        stats.succ_calls = self.succ_calls - calls0;
        stats.prepare_ms = t.elapsed().as_secs_f64() * 1e3;
        Prep { succ_old, roots_old, imports, hc, stats }
    }

    /// After the changes are applied: the new edges of the dirty states, the reached set, the
    /// findings.
    pub fn finish(&mut self, f: &Facts, p: Prep) -> Stats {
        let t = std::time::Instant::now();
        let calls0 = self.succ_calls;
        let Prep { succ_old, roots_old, imports, mut hc, mut stats } = p;
        for (target, module_kind, added) in imports {
            self.count_import(target, module_kind, if added { 1 } else { -1 });
        }
        let mut seeds = Vec::new();
        let mut work: BinaryHeap<Reverse<(u32, St)>> = BinaryHeap::new();
        for (x, old) in roots_old {
            let new = self.is_root(f, x);
            if old && !new {
                seeds.push(st(x, NONE));
                stats.roots_deleted += 1;
            }
            if new && !old {
                work.push(Reverse((0, st(x, NONE))));
                stats.roots_inserted += 1;
            }
        }
        let mut inserted: Vec<(St, St)> = Vec::new();
        let mut succ_new: FxHashMap<St, Vec<St>> = FxHashMap::default();
        for (s, old) in &succ_old {
            let new = self.succ_sorted(f, *s);
            for t in old {
                if new.binary_search(t).is_err() {
                    seeds.push(*t);
                    stats.edges_deleted += 1;
                }
            }
            for t in &new {
                if old.binary_search(t).is_err() {
                    inserted.push((*s, *t));
                    stats.edges_inserted += 1;
                }
            }
            succ_new.insert(*s, new);
        }
        // Unsupported closure. Every reached state that is not a root keeps a predecessor of
        // lower level whose edge still exists (levels are the well-founded witness that breaks
        // cycles). A deleted edge's target or a deleted root loses its place only if no such
        // predecessor is left; then its successors of higher level are checked in turn. In
        // increasing level, so every state that could support one checked is already settled.
        let mut heap: BinaryHeap<Reverse<(u32, St)>> = seeds.into_iter().filter_map(|s| self.reached.get(&s).map(|&l| Reverse((l, s)))).collect();
        let mut unsup: FxHashSet<St> = FxHashSet::default();
        let mut pc = 0usize;
        while let Some(Reverse((l, v))) = heap.pop() {
            if unsup.contains(&v) || self.reached.get(&v) != Some(&l) {
                continue;
            }
            if self.support(f, v, Some(l), &unsup, &succ_new, &mut pc).is_some() {
                continue;
            }
            unsup.insert(v);
            let next = match succ_old.get(&v) {
                Some(x) => x.clone(),
                None => self.succ_sorted(f, v),
            };
            for w in next {
                if let Some(&lw) = self.reached.get(&w) {
                    if lw > l && !unsup.contains(&w) {
                        heap.push(Reverse((lw, w)));
                    }
                }
            }
        }
        stats.over_deleted = unsup.len();
        for &s in &unsup {
            self.remove(s);
        }
        // Re-derive: an unsupported state with a root or any reached predecessor (its lowest
        // level + 1), then forward through the new edges.
        let none = FxHashSet::default();
        for &s in &unsup {
            if let Some(l) = self.support(f, s, None, &none, &succ_new, &mut pc) {
                work.push(Reverse((l, s)));
                stats.rederived += 1;
            }
        }
        stats.pred_checks = pc;
        for (s, t) in inserted {
            if let Some(&l) = self.reached.get(&s) {
                if !self.reached.contains_key(&t) {
                    work.push(Reverse((l + 1, t)));
                }
            }
        }
        let before = unsup;
        let added = self.propagate(f, work);
        stats.propagated = added.len();
        // States whose reached status changed: unsupported and not back, or newly reached.
        let back: FxHashSet<St> = added.iter().copied().filter(|s| before.contains(s)).collect();
        let toggled: Vec<St> = before.iter().copied().filter(|s| !back.contains(s)).chain(added.iter().copied().filter(|s| !before.contains(s))).collect();
        stats.toggled = toggled.len();
        for s in toggled {
            if layer(s) == ANN {
                let x = node(s);
                hc.insert(x);
                for &(o, _) in at(&f.hier_by_base, &x) {
                    Self::hc_owner(f, o, &mut hc);
                }
            }
        }
        stats.findings_evaluated = hc.len();
        for id in hc {
            let new = match f.defs.get(&id) {
                Some(d) => self.finding(f, d),
                None => Vec::new(),
            };
            let old = self.findings.remove(&id).unwrap_or_default();
            if old != new {
                stats.findings_changed += 1;
                note(&mut stats.delta, &old, &new);
            }
            if !new.is_empty() {
                self.findings.insert(id, new);
            }
        }
        stats.succ_calls += self.succ_calls - calls0;
        stats.finish_ms = t.elapsed().as_secs_f64() * 1e3;
        stats
    }

    fn ann(&self, x: u32) -> bool {
        self.reached.contains_key(&st(x, ANN))
    }

    /// `dc__high_confidence` and `dc__findings` at one definition: a local query over its
    /// definition, its class, its hierarchy rows and the reached set at those nodes.
    fn finding(&self, f: &Facts, d: &Def) -> Vec<Finding> {
        if !self.high_confidence(f, d) {
            return Vec::new();
        }
        let path = string(d.path);
        if path.starts_with("impl/browser/") {
            return Vec::new();
        }
        let row = |c: &'static str| (d.path, d.start, d.end, d.id, c);
        if d.kind != Kind::Class {
            return vec![row("high_confidence_functions")];
        }
        let (pi, pg) = f.probe.get(&(d.path, d.start)).copied().unwrap_or((false, false));
        let intent = with(d.name, |n| self.rx.intent.is_match(n)) || pi;
        let grammar = !d.protocol && (path.contains("/l2100_sql_grammar/") || with(d.name, |n| self.rx.grammar.is_match(n)) || pg);
        let mut v = Vec::new();
        if intent {
            v.push(row("high_confidence_intents"));
        }
        if grammar {
            v.push(row("high_confidence_grammar_classes"));
        }
        if !intent && !grammar {
            v.push(row("high_confidence_other_classes"));
        }
        v
    }

    fn high_confidence(&self, f: &Facts, d: &Def) -> bool {
        if self.ann(d.id) || with(d.module, |m| self.rx.api_module.is_match(m)) || d.implicit {
            return false;
        }
        if at(&f.imports_by_target, &d.fqname).iter().any(|i| i.symbol_kind) {
            return false;
        }
        if d.kind == Kind::Class && d.protocol {
            return false;
        }
        if d.kind != Kind::Class {
            let local = d.abstract_ || d.property || with(d.name, |n| n.starts_with("_lower_") || n.to_ascii_lowercase().contains("register") || self.rx.create.is_match(n));
            if local || d.class_id.and_then(|c| f.defs.get(&c)).is_some_and(|c| c.protocol) {
                return false;
            }
            if let Some(cid) = d.class_id {
                for &(b, depth) in at(&f.hier_by_owner, &cid) {
                    if depth > 0 {
                        if let Some(c) = f.defs.get(&b) {
                            if at(&f.by_module_class, &(c.module, c.name)).iter().any(|m| f.defs[m].name == d.name && f.defs[m].abstract_) {
                                return false;
                            }
                        }
                    }
                }
            }
        }
        let owner = if d.kind == Kind::Class { Some(d.id) } else { d.class_id };
        if let Some(o) = owner {
            for &(b, depth) in at(&f.hier_by_owner, &o) {
                if depth > 0 && self.ann(b) {
                    if let Some(bd) = f.defs.get(&b) {
                        if bd.abstract_ || (with(bd.name, |n| n.contains("ABC")) && at(&f.bases_by_owner, &o).contains(&b)) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// `dc__findings` rows as the dump prints them.
    pub fn finding_rows(&self) -> Vec<Vec<Val>> {
        let mut v: Vec<Vec<Val>> = self.findings.values().flatten().map(|(p, s, e, id, c)| vec![Val::S(*p), Val::Int(*s), Val::Int(*e), Val::S(*id), Val::S(crate::y2::facts::sym(c))]).collect();
        v.sort();
        v
    }
}

pub fn node_string(n: u32) -> String {
    if n & MOD != 0 {
        format!("@module:{}", string(n & !MOD))
    } else {
        string(n)
    }
}

fn succ(f: &Facts, s: St, out: &mut Vec<St>) {
    let (x, l) = (node(s), layer(s));
    if l == ANN {
        for &a in at(&f.ann_by_symbol, &x) {
            if let Some(t) = unique_class(f, a) {
                out.push(st(t, ANN));
            }
        }
        return;
    }
    if l >= 2 {
        let dsym = l - 2;
        for &o in at(&f.bases_by_base, &x) {
            out.push(st(o, l));
        }
        if let Some(od) = f.defs.get(&dsym) {
            if let Some(ocid) = od.class_id {
                for &b in at(&f.bases_by_owner, &x) {
                    if b != ocid {
                        out.push(st(b, l));
                    }
                }
            }
            if let Some(c) = f.defs.get(&x) {
                for &m in at(&f.by_module_class, &(c.module, c.name)) {
                    if f.defs[&m].name == od.name {
                        out.push(st(m, NONE));
                    }
                }
            }
        }
        return;
    }
    out.push(st(x, ANN));
    if x & MOD != 0 {
        let m = x & !MOD;
        for &nm in at(&f.module_names, &m) {
            let local = at(&f.by_module_name, &(m, nm));
            for &d in local {
                out.push(st(d, NONE));
            }
            if local.is_empty() {
                for i in at(&f.imports_by_module_name, &(m, nm)) {
                    if i.symbol_kind {
                        for &d in at(&f.by_fqname, &i.target) {
                            out.push(st(d, NONE));
                        }
                    }
                }
            }
        }
        return;
    }
    for &t in at(&f.public_by_owner, &x) {
        out.push(st(t, NONE));
    }
    for &(b, _) in at(&f.hier_by_owner, &x) {
        if let Some(c) = f.defs.get(&b) {
            for &m in at(&f.by_module_class, &(c.module, c.name)) {
                if with(f.defs[&m].name, |n| CTOR.contains(&n)) {
                    out.push(st(m, NONE));
                }
            }
        }
    }
    for &b in at(&f.bases_by_owner, &x) {
        out.push(st(b, NONE));
    }
    for &o in at(&f.bases_by_base, &x) {
        out.push(st(o, NONE));
    }
    let Some(d) = f.defs.get(&x) else { return };
    for t in at(&f.refs_from, &(d.path, d.start)) {
        if let Some(&tid) = f.by_pos.get(t) {
            out.push(st(tid, NONE));
        }
    }
    let variants = with(d.base_name, |b| [b.to_owned(), format!("{b}_async"), format!("{b}_lazy")]);
    if d.kind == Kind::Function {
        for v in &variants {
            if let Some(v) = get(v) {
                for &t in at(&f.by_module_name, &(d.module, v)) {
                    if t != x {
                        out.push(st(t, NONE));
                    }
                }
            }
        }
    }
    if d.kind == Kind::Method {
        if let Some(cid) = d.class_id {
            for v in &variants {
                if let Some(t) = lookup_concat(cid, ".", v) {
                    if t != x && f.defs.contains_key(&t) {
                        out.push(st(t, NONE));
                    }
                }
            }
        }
    }
    for &a in at(&f.ann_by_symbol, &x) {
        if let Some(t) = class_target(f, d.module, a) {
            out.push(st(t, NONE));
        }
    }
    out.push(st(d.module | MOD, NONE));
    if let Some(cid) = d.class_id {
        if f.defs.get(&cid).is_some_and(|c| c.structural_protocol) {
            for &o in at(&f.methods_by_name, &d.name) {
                if f.defs[&o].class_id != Some(cid) {
                    out.push(st(o, NONE));
                }
            }
        }
    }
    if d.kind == Kind::Method {
        if let Some(cid) = d.class_id {
            for &o in at(&f.bases_by_base, &cid) {
                out.push(st(o, x + 2));
            }
        }
    }
}

/// A superset of `s`'s in-edges' sources (each is verified against `succ` by the caller).
fn pred(dc: &Dc, f: &Facts, s: St, out: &mut Vec<St>) {
    let (y, l) = (node(s), layer(s));
    if l == ANN {
        out.push(st(y, NONE));
        if let Some(d) = f.defs.get(&y) {
            if d.kind == Kind::Class {
                out.extend(at(&f.ann_by_name, &d.name).iter().map(|&a| st(a, ANN)));
            }
        }
        return;
    }
    if l >= 2 {
        out.push(st(l - 2, NONE));
        out.extend(at(&f.bases_by_owner, &y).iter().map(|&b| st(b, l)));
        out.extend(at(&f.bases_by_base, &y).iter().map(|&o| st(o, l)));
        return;
    }
    if y & MOD != 0 {
        out.extend(at(&f.by_module, &(y & !MOD)).iter().map(|&d| st(d, NONE)));
        return;
    }
    out.extend(at(&f.public_by_symbol, &y).iter().map(|&o| st(o, NONE)));
    out.extend(at(&f.bases_by_base, &y).iter().map(|&o| st(o, NONE)));
    out.extend(at(&f.bases_by_owner, &y).iter().map(|&b| st(b, NONE)));
    let Some(d) = f.defs.get(&y) else { return };
    for src in at(&f.refs_to, &(d.path, d.start)) {
        if let Some(&s) = f.by_pos.get(src) {
            out.push(st(s, NONE));
        }
    }
    if let Some(c) = d.class_name {
        for &cid in at(&f.by_module_name, &(d.module, c)) {
            out.extend(at(&f.hier_by_base, &cid).iter().map(|&(o, _)| st(o, NONE)));
            out.extend(at(&dc.disp_by_node, &cid).iter().map(|&l| st(cid, l)));
        }
    }
    with(d.name, |name| {
        for b in base_cands(name) {
            if let Some(b) = get(b) {
                out.extend(at(&f.functions_by_module_base, &(d.module, b)).iter().map(|&x| st(x, NONE)));
            }
        }
    });
    let id = string(y);
    for (k, _) in id.match_indices('.') {
        if let Some(p) = get(&id[..k]) {
            for b in base_cands(&id[k + 1..]) {
                if let Some(b) = get(b) {
                    out.extend(at(&f.methods_by_class_base, &(p, b)).iter().map(|&x| st(x, NONE)));
                }
            }
        }
    }
    if d.kind == Kind::Class {
        let mut keys = vec![(d.module, d.name)];
        for fq in at(&f.resolved_by_symbol, &y) {
            for i in at(&f.imports_by_target, fq) {
                if i.symbol_kind {
                    keys.push((i.module, i.name));
                }
            }
        }
        for (m, n) in keys {
            for &a in at(&f.ann_by_name, &n) {
                if f.defs.get(&a).is_some_and(|x| x.module == m) {
                    out.push(st(a, NONE));
                }
            }
        }
    }
    out.push(st(d.module | MOD, NONE));
    for i in at(&f.imports_by_target, &d.fqname) {
        out.push(st(i.module | MOD, NONE));
    }
    if d.kind == Kind::Method {
        out.extend(at(&f.methods_by_name, &d.name).iter().map(|&m| st(m, NONE)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_last_is_the_rules_rx_replace() {
        // rx_replace(s, '\.?[^.]+$', '')
        assert_eq!(strip_last("a.b.c"), "a.b");
        assert_eq!(strip_last("a"), "");
        assert_eq!(strip_last("a.b."), "a.b.");
        assert_eq!(strip_last(""), "");
    }

    #[test]
    fn import_chain_is_imported_modules() {
        assert_eq!(import_chain(crate::y2::facts::sym("p.q.r"), true), ["p.q.r", "p.q", "p"]);
        assert_eq!(import_chain(crate::y2::facts::sym("p.q.Name"), false), ["p.q", "p"]);
        assert!(import_chain(crate::y2::facts::sym("Name"), false).is_empty());
    }

    #[test]
    fn base_cands_invert_the_sibling_names() {
        // s.name IN (base_name, base_name || '_async', base_name || '_lazy')
        assert_eq!(base_cands("f"), ["f"]);
        assert_eq!(base_cands("f_async"), ["f_async", "f"]);
        assert_eq!(base_cands("f_lazy"), ["f_lazy", "f"]);
    }
}
