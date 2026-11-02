//! `unreachable-internal-symbols` (dead code) as local queries over the graph, with reachability
//! the one property that is not local.
//!
//! The rule's `reachable` and `annotation_reachable` are one reachability over a product graph
//! whose states are (node, layer): layer `NONE` is the rule's `flow(node, NULL)`, layer `d + 2`
//! is `flow(node, d)` (a dispatch search for method `d` through the class hierarchy), layer `ANN`
//! is `annotated(node)`. Every edge of that graph is a local query (`succ`): it reads the source
//! node's definition, its reference edges (`refs_from`), and keyed symbol-table rows. Roots are a
//! local predicate (`Rule::is_root`). What is *not* local is the reached set: "reachable from any
//! root" changes for an unbounded set of states when one edge goes (a chain behind it).
//!
//! In facts-b, Y2 maintained the reached set itself (levels as witnesses, `pred`, `finish`). Here
//! the reached set is the lattice fact `facts::deadcode` maintains (E3's `min_level`) or recomputes
//! (E1's fixpoint) over the product graph's CSR, so Y2 keeps: `succ` (the CSR's rows), the roots
//! over the maintained imported-module counts (`Rule`), the scoping (`scope`), and the findings as
//! a function of the facts and the reached annotated states (`finding`). `Dc` is Y2's reachability
//! and findings from scratch, kept as the exactness oracle.
//!
//! Scoping (exact; see `scope`): an edit's presence changes name keys; `scope` collects, under the
//! old state, every node whose states' `succ` or root predicate read one of those keys (inverse
//! indexes), the dispatch layers of changed definitions, and every definition whose finding reads
//! one. A state's `succ` is a deterministic function of the keys it reads, so a state outside
//! that set has the same edges before and after. The same holds for a finding, plus its reads of
//! the reached set, which the caller adds from the annotated states whose reached status toggled
//! (`toggled_readers`).
use super::facts::{at, get, note, string, with, Change, Def, Facts, Kind, Tab, Val};
use super::super::lmap::LMap;
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub type St = u64;
pub(crate) const NONE: u32 = 0;
pub(crate) const ANN: u32 = 1;
pub(crate) const MOD: u32 = 1 << 31;

pub(crate) fn st(node: u32, layer: u32) -> St {
    ((node as u64) << 32) | layer as u64
}
pub(crate) fn node(s: St) -> u32 {
    (s >> 32) as u32
}
pub(crate) fn layer(s: St) -> u32 {
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

/// What one edit's presence changes reach, under the old state (`scope`).
#[derive(Default, Debug)]
pub struct Scope {
    /// Nodes all of whose states may have other out-edges after the edit (sorted, unique).
    pub nodes: Vec<u32>,
    /// Dispatch layers `d + 2` of changed definitions `d` (sorted, unique): every state of the
    /// layer may (`flow(n, d)` reads `d`'s name and class).
    pub layers: Vec<u32>,
    /// Nodes whose root predicate may change (sorted, unique).
    pub roots: Vec<u32>,
    /// Definitions whose finding reads a changed key.
    pub findings: FxHashSet<u32>,
    /// Import rows that left and came: (target, module kind, added), for the imported-module
    /// counts the roots read (`Rule::apply`).
    imports: Vec<(u32, bool, bool)>,
    pub changes: usize,
    pub ms: f64,
}

pub type Finding = (u32, i64, i64, u32, &'static str);

/// Dead code's maintained state besides reachability: the imported-module counts its roots read.
/// Layered (`lmap`): a worktree's `Rule` starts as a view of its layer's and holds what differs.
pub struct Rule {
    rx: Rx,
    imported: LMap<u32, u32>,
}

impl Rule {
    /// From scratch: every import row's contribution (`Dc::seed`'s counts).
    pub fn scratch(f: &Facts) -> Rule {
        let mut imported = LMap::default();
        for is in f.imports_by_target.values() {
            for i in is {
                count_import(&mut imported, i.target, !i.symbol_kind, 1);
            }
        }
        Rule { rx: Rx::new(), imported }
    }

    /// A worktree's state over this (frozen) one.
    pub fn view(&self) -> Rule {
        Rule { rx: self.rx.clone(), imported: self.imported.view() }
    }

    pub fn freeze(&mut self) {
        self.imported.freeze();
    }

    /// Bytes of the top (this worktree's difference) and of the top-most frozen layer.
    pub fn bytes(&self) -> (usize, usize) {
        (self.imported.top_bytes(), self.imported.layer_bytes(0))
    }

    pub(crate) fn is_root(&self, f: &Facts, x: u32) -> bool {
        is_root(&self.imported, f, x)
    }

    /// The imported-module counts after `s`'s changes (applied to the facts).
    pub fn apply(&mut self, s: &Scope) {
        for &(target, module_kind, added) in &s.imports {
            count_import(&mut self.imported, target, module_kind, if added { 1 } else { -1 });
        }
    }

    /// `d`'s findings, with `ann(x)` the reached status of `annotated(x)`.
    pub fn finding(&self, f: &Facts, d: &Def, ann: &dyn Fn(u32) -> bool) -> Vec<Finding> {
        finding(&self.rx, f, d, ann)
    }
}

fn count_import(imported: &mut LMap<u32, u32>, target: u32, module_kind: bool, d: i32) {
    for m in import_chain(target, module_kind) {
        let k = super::facts::sym(&m);
        let c = (imported.get(&k).copied().unwrap_or(0) as i32 + d) as u32;
        if c == 0 {
            imported.remove(&k);
        } else {
            imported.insert(k, c);
        }
    }
}

fn is_root(imported: &LMap<u32, u32>, f: &Facts, x: u32) -> bool {
    if x & MOD != 0 {
        return imported.contains_key(&(x & !MOD));
    }
    f.roots_by_symbol.contains_key(&x) || at(&f.public_by_symbol, &x).iter().any(|o| f.roots_by_symbol.contains_key(o)) || at(&f.resolved_by_symbol, &x).iter().any(|fq| f.dyn_fq.contains_key(fq))
}

fn readers_class_target(f: &Facts, m: u32, n: u32, out: &mut FxHashSet<u32>) {
    for &s in at(&f.ann_by_name, &n) {
        if f.defs.get(&s).is_some_and(|d| d.module == m) {
            out.insert(s);
        }
    }
}

/// Definitions whose finding reads class `o`'s hierarchy or base rows: `o` and its methods.
fn hc_owner(f: &Facts, o: u32, hc: &mut FxHashSet<u32>) {
    hc.insert(o);
    hc.extend(at(&f.methods_by_class, &o).iter().copied());
}

/// Definitions whose finding reads the reached status of `annotated(x)` (facts-b's `finish`, for
/// a toggled annotated state): `x`, and the owners (with their methods) of classes `x` is a base
/// of.
pub fn toggled_readers(f: &Facts, x: u32, out: &mut FxHashSet<u32>) {
    out.insert(x);
    for &(o, _) in at(&f.hier_by_base, &x) {
        hc_owner(f, o, out);
    }
}

/// Under the old state (nothing applied): the nodes whose states' `succ` or root predicate, and
/// the definitions whose finding, read a key these presence changes touch (facts-b's
/// `Dc::prepare` up to its scope, without the reached set: a dirty node stands for all its
/// states, and a changed definition's dispatch layer for all of its states).
pub fn scope(f: &Facts, changes: &[Change]) -> Scope {
    let t = std::time::Instant::now();
    let mut d: FxHashSet<u32> = FxHashSet::default();
    let mut rc: FxHashSet<u32> = FxHashSet::default();
    let mut hc: FxHashSet<u32> = FxHashSet::default();
    let mut imports = Vec::new();
    let mut layers: Vec<u32> = Vec::new();
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
        d.insert(*id);
        for x in [a.as_ref().unwrap(), b.as_ref().unwrap()] {
            for src in at(&f.refs_to, &(x.path, x.start)) {
                if let Some(&s) = f.by_pos.get(src) {
                    d.insert(s);
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
                d.insert(x.id);
                layers.push(x.id + 2);
                for &(o, _) in at(&f.hier_by_base, &x.id) {
                    d.insert(o);
                    hc_owner(f, o, &mut hc);
                }
                if let Some(c) = x.class_name {
                    for &cid in at(&f.by_module_name, &(x.module, c)) {
                        d.insert(cid);
                        for &(o, _) in at(&f.hier_by_base, &cid) {
                            d.insert(o);
                            hc_owner(f, o, &mut hc);
                        }
                    }
                }
                for src in at(&f.refs_to, &(x.path, x.start)) {
                    if let Some(&s) = f.by_pos.get(src) {
                        d.insert(s);
                    }
                }
                with(x.name, |name| {
                    for b in base_cands(name) {
                        if let Some(b) = get(b) {
                            for &s in at(&f.functions_by_module_base, &(x.module, b)) {
                                d.insert(s);
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
                                    d.insert(s);
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
                    readers_class_target(f, m, n, &mut d);
                }
                d.insert(x.module | MOD);
                for i in at(&f.imports_by_target, &x.fqname) {
                    d.insert(i.module | MOD);
                }
                // E11: methods of `x` if `x` is a structural protocol; methods named like
                // `x` whose class is one (they read the list `x` is in).
                if x.structural_protocol {
                    for &m in at(&f.methods_by_class, &x.id) {
                        d.insert(m);
                    }
                }
                if x.kind == Kind::Method {
                    for &m in at(&f.methods_by_name, &x.name) {
                        if f.defs[&m].class_id.and_then(|c| f.defs.get(&c)).is_some_and(|c| c.structural_protocol) {
                            d.insert(m);
                        }
                    }
                }
                hc_owner(f, x.id, &mut hc);
            }
            Tab::Refs => {
                if let Some(&s) = f.by_pos.get(&(r[0].s(), r[1].i())) {
                    d.insert(s);
                }
            }
            Tab::Bases => {
                let (o, b) = (r[0].s(), r[1].s());
                d.insert(o);
                d.insert(b);
                for &m in at(&f.methods_by_class, &b) {
                    d.insert(m);
                }
                hc_owner(f, o, &mut hc);
            }
            Tab::Hier => {
                d.insert(r[0].s());
                hc_owner(f, r[0].s(), &mut hc);
            }
            Tab::Public => {
                d.insert(r[0].s());
                rc.insert(r[2].s());
            }
            Tab::Roots => {
                let s = r[3].s();
                rc.insert(s);
                rc.extend(at(&f.public_by_owner, &s).iter().copied());
            }
            Tab::Imports => {
                let (m, n, target, module_kind) = (r[3].s(), r[4].s(), r[5].s(), with(r[6].s(), |k| k == "module"));
                d.insert(m | MOD);
                readers_class_target(f, m, n, &mut d);
                for c in import_chain(target, module_kind) {
                    rc.insert(super::facts::sym(&c) | MOD);
                }
                hc.extend(at(&f.by_fqname, &target).iter().copied());
                imports.push((target, module_kind, ch.added));
            }
            Tab::Resolved => {
                let (fq, s) = (r[0].s(), r[1].s());
                for i in at(&f.imports_by_target, &fq) {
                    if i.symbol_kind {
                        readers_class_target(f, i.module, i.name, &mut d);
                    }
                }
                rc.insert(s);
            }
            Tab::Ann => {
                d.insert(r[0].s());
            }
            Tab::ModuleNames => {
                d.insert(r[0].s() | MOD);
            }
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
                d.insert(s);
            }
        }
    }
    let mut nodes: Vec<u32> = d.into_iter().collect();
    nodes.sort_unstable();
    layers.sort_unstable();
    layers.dedup();
    let mut roots: Vec<u32> = rc.into_iter().collect();
    roots.sort_unstable();
    Scope { nodes, layers, roots, findings: hc, imports, changes: changes.len(), ms: t.elapsed().as_secs_f64() * 1e3 }
}

/// `dc__high_confidence` and `dc__findings` at one definition: a local query over its definition,
/// its class, its hierarchy rows and the reached set at those nodes (`ann`).
fn finding(rx: &Rx, f: &Facts, d: &Def, ann: &dyn Fn(u32) -> bool) -> Vec<Finding> {
    if !high_confidence(rx, f, d, ann) {
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
    let intent = with(d.name, |n| rx.intent.is_match(n)) || pi;
    let grammar = !d.protocol && (path.contains("/l2100_sql_grammar/") || with(d.name, |n| rx.grammar.is_match(n)) || pg);
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

fn high_confidence(rx: &Rx, f: &Facts, d: &Def, ann: &dyn Fn(u32) -> bool) -> bool {
    if ann(d.id) || with(d.module, |m| rx.api_module.is_match(m)) || d.implicit {
        return false;
    }
    if at(&f.imports_by_target, &d.fqname).iter().any(|i| i.symbol_kind) {
        return false;
    }
    if d.kind == Kind::Class && d.protocol {
        return false;
    }
    if d.kind != Kind::Class {
        let local = d.abstract_ || d.property || with(d.name, |n| n.starts_with("_lower_") || n.to_ascii_lowercase().contains("register") || rx.create.is_match(n));
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
            if depth > 0 && ann(b) {
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

/// `dc__findings` rows as the dump prints them, from findings by definition.
pub fn finding_rows<'a>(findings: impl Iterator<Item = &'a Vec<Finding>>) -> Vec<Vec<Val>> {
    let mut v: Vec<Vec<Val>> = findings.flatten().map(|(p, s, e, id, c)| vec![Val::S(*p), Val::Int(*s), Val::Int(*e), Val::S(*id), Val::S(super::facts::sym(c))]).collect();
    v.sort();
    v
}

/// Y2's dead code from scratch (facts-b's `Dc::scratch`): roots, reachability by a level-ordered
/// search, every definition's finding over that reached set. The exactness oracle for the
/// dead-code producer.
pub struct Dc {
    rx: Rx,
    /// The reached states with their levels (BFS depth).
    reached: FxHashMap<St, u32>,
    imported: LMap<u32, u32>,
    pub findings: FxHashMap<u32, Vec<Finding>>,
}

impl Dc {
    fn empty() -> Dc {
        Dc { rx: Rx::new(), reached: FxHashMap::default(), imported: LMap::default(), findings: FxHashMap::default() }
    }

    /// Reached states.
    pub fn reached_len(&self) -> usize {
        self.reached.len()
    }

    /// From scratch: every root, reachability, every definition's finding.
    pub fn scratch(f: &Facts) -> Dc {
        let mut dc = Dc::scratch_reach(f);
        dc.scratch_findings(f);
        dc
    }

    /// From scratch, reachability alone (roots, then `propagate` from them).
    pub fn scratch_reach(f: &Facts) -> Dc {
        let mut dc = Dc::empty();
        let roots = dc.seed(f);
        let work: BinaryHeap<Reverse<(u32, St)>> = roots.into_iter().map(|r| Reverse((0, r))).collect();
        dc.propagate(f, work);
        dc
    }

    /// From scratch, every definition's finding over the reached set: the second half of `scratch`.
    pub fn scratch_findings(&mut self, f: &Facts) {
        let ann = |x: u32| self.reached.contains_key(&st(x, ANN));
        let mut out = FxHashMap::default();
        for (&id, d) in f.defs.iter() {
            let v = finding(&self.rx, f, d, &ann);
            if !v.is_empty() {
                out.insert(id, v);
            }
        }
        self.findings = out;
    }

    /// The imported-module counts, then the root states (layer `NONE`, possibly repeated).
    fn seed(&mut self, f: &Facts) -> Vec<St> {
        for is in f.imports_by_target.values() {
            for i in is {
                count_import(&mut self.imported, i.target, !i.symbol_kind, 1);
            }
        }
        let mut roots: Vec<u32> = f.roots_by_symbol.keys().copied().collect();
        for r in f.roots_by_symbol.keys() {
            roots.extend(at(&f.public_by_owner, r).iter().copied());
        }
        roots.extend(self.imported.keys().map(|m| m | MOD));
        for fq in f.dyn_fq.keys() {
            roots.extend(at(&f.resolved_by_fq, fq).iter().copied());
        }
        roots.into_iter().map(|r| st(r, NONE)).collect()
    }

    /// The root states of a from-scratch reachability, as `scratch_reach` seeds them.
    pub fn root_states(f: &Facts) -> Vec<St> {
        Dc::empty().seed(f)
    }

    /// Every reached state with its level.
    pub fn levels(&self) -> Vec<(St, u32)> {
        self.reached.iter().map(|(s, l)| (*s, *l)).collect()
    }

    /// Forward from `work` (state, level), lowest level first.
    fn propagate(&mut self, f: &Facts, mut work: BinaryHeap<Reverse<(u32, St)>>) {
        let mut buf = Vec::new();
        while let Some(Reverse((l, s))) = work.pop() {
            if self.reached.contains_key(&s) {
                continue;
            }
            self.reached.insert(s, l);
            buf.clear();
            succ(f, s, &mut buf);
            for &t in &buf {
                if !self.reached.contains_key(&t) {
                    work.push(Reverse((l + 1, t)));
                }
            }
        }
    }

    /// `dc__findings` rows as the dump prints them.
    pub fn finding_rows(&self) -> Vec<Vec<Val>> {
        finding_rows(self.findings.values())
    }
}

pub fn node_string(n: u32) -> String {
    if n & MOD != 0 {
        format!("@module:{}", string(n & !MOD))
    } else {
        string(n)
    }
}

/// The product graph's out-edges of `s` (possibly repeated): local queries at `s`'s node.
pub(crate) fn succ(f: &Facts, s: St, out: &mut Vec<St>) {
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
        assert_eq!(import_chain(super::super::facts::sym("p.q.r"), true), ["p.q.r", "p.q", "p"]);
        assert_eq!(import_chain(super::super::facts::sym("p.q.Name"), false), ["p.q", "p"]);
        assert!(import_chain(super::super::facts::sym("Name"), false).is_empty());
    }

    #[test]
    fn base_cands_invert_the_sibling_names() {
        // s.name IN (base_name, base_name || '_async', base_name || '_lazy')
        assert_eq!(base_cands("f"), ["f"]);
        assert_eq!(base_cands("f_async"), ["f_async", "f"]);
        assert_eq!(base_cands("f_lazy"), ["f_lazy", "f"]);
    }
}
