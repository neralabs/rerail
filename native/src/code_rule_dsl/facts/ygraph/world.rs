//! Base and worktree state, and the per-event maintenance step: per-path blocks deleted and
//! reinserted, keyed tables updated with the changed keys collected, the readers of those keys
//! re-run in level order (engine), roots registered, refusals and edges updated.
use super::engine::*;
use super::intern::{Interner, Sym};
use super::path::{self, Block, ModuleCtx, SDef, CLASS};
use super::summary::Summaries;
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Instant;

/// Definition ids: one per (path, def name, occurrence of the name in the path), shared by the
/// base and every worktree. A def keeps its id while its path's content changes around it (its
/// idx and span are read through the worktree's view).
#[derive(Default)]
pub struct DefTable {
    pub map: FxHashMap<(u32, u32, u32), u32>,
    pub defs: Vec<(u32, u32, u32)>,
}

impl DefTable {
    pub fn id(&mut self, path: u32, name: u32, occ: u32) -> u32 {
        if let Some(&d) = self.map.get(&(path, name, occ)) {
            return d;
        }
        let d = self.defs.len() as u32;
        self.defs.push((path, name, occ));
        self.map.insert((path, name, occ), d);
        d
    }
    pub fn get(&self, path: u32, name: u32, occ: u32) -> Option<u32> {
        self.map.get(&(path, name, occ)).copied()
    }
    pub fn bytes(&self) -> usize {
        self.map.capacity() * 17 + self.defs.capacity() * 12
    }
}

/// The rule's scope (`scope_glob`, `in_scope`).
pub struct Scope {
    include: Vec<regex::Regex>,
    exclude: Vec<regex::Regex>,
}

impl Scope {
    pub fn load(rule: &std::path::Path) -> Scope {
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(rule).unwrap()).unwrap();
        let re = |g: &str| {
            let r = g.replace('.', "\\.").replace("**/", "%DIRS%").replace("/**", "%BELOW%").replace('*', "[^/]*");
            regex::Regex::new(&format!("^{}$", r.replace("%DIRS%", "(?:.*/)?").replace("%BELOW%", "/.*"))).unwrap()
        };
        let list = |k: &str| doc["scope"][k].as_array().map(|a| a.iter().map(|g| re(g.as_str().unwrap())).collect()).unwrap_or_default();
        Scope { include: list("include"), exclude: list("exclude") }
    }
    pub fn admits(&self, p: &str) -> bool {
        self.include.iter().any(|r| r.is_match(p)) && !self.exclude.iter().any(|r| r.is_match(p))
    }
}

/// Everything shared by the base and the worktrees: strings, inputs, query and def ids.
pub struct Shared {
    pub it: Interner,
    pub sums: Summaries,
    pub qt: QTable,
    pub defs: DefTable,
    pub anomalies: Anomalies,
    pub scope: Scope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRec {
    pub content: Sym,
    pub ctx: ModuleCtx,
    pub in_scope: bool,
    pub linked: bool,
    pub refused: bool,
    pub open: bool,
    pub module: Option<Sym>,
    pub defs: Box<[SDef]>,
    pub edges: Box<[(u32, Sym)]>,
}

impl PathRec {
    fn bytes(&self) -> usize {
        std::mem::size_of::<PathRec>()
            + self.ctx.segments.as_ref().map_or(0, |s| s.iter().map(|x| x.capacity() + 24).sum())
            + self.defs.len() * std::mem::size_of::<SDef>()
            + self.edges.len() * 8
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Counters {
    pub refused_paths: u32,
    pub open_paths: u32,
    pub refusal_roots: u32,
}

/// A frozen layer: the base graph state, or (overlay) a cluster's rows that differ from the
/// layers below it. Overlay entries override: a None path, an empty edge list, an empty table
/// key and a false contribution all mean "not here", whatever lies below.
#[derive(Default)]
pub struct Base {
    pub tables: BaseTables,
    pub memos: BaseMemos,
    pub paths: FxHashMap<Sym, Option<PathRec>>,
    pub pkg: FxHashMap<String, u32>,
    pub out: FxHashMap<Sym, Box<[(u32, u32)]>>,
    pub counters: Counters,
    pub contrib: FxHashMap<QId, bool>,
}

/// The frozen layers under a worktree, top first: its cluster's overlay (if any), the base.
#[derive(Clone, Default)]
pub struct Below<'a> {
    pub layers: Vec<&'a Base>,
}

impl<'a> Below<'a> {
    pub fn new(layers: Vec<&'a Base>) -> Self {
        Below { layers }
    }
    pub fn rec(&self, p: Sym) -> Option<&'a PathRec> {
        self.layers.iter().find_map(|l| l.paths.get(&p)).and_then(Option::as_ref)
    }
    pub fn out(&self, p: Sym) -> &'a [(u32, u32)] {
        self.layers.iter().find_map(|l| l.out.get(&p)).map_or(&[], |o| o)
    }
    pub fn contrib(&self, q: QId) -> bool {
        self.layers.iter().find_map(|l| l.contrib.get(&q)).copied().unwrap_or(false)
    }
    pub fn tables(&self) -> Vec<&'a BaseTables> {
        self.layers.iter().map(|l| &l.tables).collect()
    }
    pub fn memos(&self) -> Vec<&'a BaseMemos> {
        self.layers.iter().map(|l| &l.memos).collect()
    }
    /// Every member path with its record, overrides applied.
    pub fn members(&self) -> FxHashMap<Sym, &'a PathRec> {
        let mut m: FxHashMap<Sym, &PathRec> = FxHashMap::default();
        for l in self.layers.iter().rev() {
            for (p, r) in &l.paths {
                match r {
                    Some(r) => {
                        m.insert(*p, r);
                    }
                    None => {
                        m.remove(p);
                    }
                }
            }
        }
        m
    }
}

/// One worktree: the rows that differ from the layers below it.
#[derive(Default, Clone)]
pub struct Delta {
    pub tables: DeltaTables,
    pub memos: DeltaMemos,
    /// None: the path is not a member here
    pub paths: FxHashMap<Sym, Option<PathRec>>,
    pub pkg: FxHashMap<String, u32>,
    pub out: FxHashMap<Sym, Box<[(u32, u32)]>>,
    pub counters: Counters,
    pub contrib: FxHashMap<QId, bool>,
    pub epoch: u64,
}

/// Counts of one event's maintenance.
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct Step {
    pub paths_changed: u32,
    pub paths_reblocked: u32,
    pub table_rows_touched: u64,
    pub keys_changed: u64,
    pub work: Work,
    pub roots_checked: u32,
    pub out_paths: u32,
    pub init_fallback: bool,
    pub incomplete_flip: bool,
    pub ms: f64,
    /// The step's change to the graph, for local rules (Y2): (source def, target def) pairs
    /// added and removed, and the paths whose definitions were re-read.
    #[serde(skip)]
    pub edges_added: Vec<(u32, u32)>,
    #[serde(skip)]
    pub edges_removed: Vec<(u32, u32)>,
    #[serde(skip)]
    pub def_paths: Vec<Sym>,
    /// The re-read paths' records before the step (None: not a member before).
    #[serde(skip)]
    pub old_recs: FxHashMap<Sym, Option<PathRec>>,
    /// Per path whose edges were recomputed: its (source def, target def) pairs before and
    /// after (Y3 turns them into position rows for Y2).
    #[serde(skip)]
    pub out_pairs: Vec<(Sym, Vec<(u32, u32)>, Vec<(u32, u32)>)>,
}

impl Delta {
    pub fn rec<'a>(&'a self, below: &Below<'a>, p: Sym) -> Option<&'a PathRec> {
        match self.paths.get(&p) {
            Some(r) => r.as_ref(),
            None => below.rec(p),
        }
    }

    pub fn out<'a>(&'a self, below: &Below<'a>, p: Sym) -> &'a [(u32, u32)] {
        match self.out.get(&p) {
            Some(o) => o,
            None => below.out(p),
        }
    }

    pub fn contrib(&self, below: &Below, q: QId) -> bool {
        match self.contrib.get(&q) {
            Some(c) => *c,
            None => below.contrib(q),
        }
    }

    pub fn members<'a>(&'a self, below: &Below<'a>) -> Vec<(Sym, &'a PathRec)> {
        let mut m = below.members();
        for (p, r) in &self.paths {
            match r {
                Some(r) => {
                    m.insert(*p, r);
                }
                None => {
                    m.remove(p);
                }
            }
        }
        m.into_iter().collect()
    }

    pub fn incomplete(&self) -> bool {
        self.counters.open_paths > 0
    }
    pub fn refused(&self) -> bool {
        self.counters.refused_paths > 0 || self.counters.refusal_roots > 0
    }

    pub fn bytes(&self) -> usize {
        self.tables.bytes()
            + self.memos.bytes()
            + self.paths.capacity() * 12 + self.paths.values().map(|r| r.as_ref().map_or(0, PathRec::bytes)).sum::<usize>()
            + self.out.capacity() * 20 + self.out.values().map(|o| o.len() * 8).sum::<usize>()
            + self.contrib.capacity() * 8
            + self.pkg.iter().map(|(k, _)| k.capacity() + 40).sum::<usize>()
    }

    /// Bytes by component, with the counts behind them.
    pub fn breakdown(&self) -> serde_json::Value {
        let m = &self.memos;
        let table_keys: usize = self.tables.t.iter().map(|t| t.len()).sum();
        let table_rows: usize = self.tables.t.iter().map(|t| t.values().map(Vec::len).sum::<usize>()).sum();
        let per_table: Vec<(usize, usize)> = self.tables.t.iter().map(|t| (t.len(), t.values().map(Vec::len).sum::<usize>())).collect();
        serde_json::json!({
            "tables": self.tables.bytes(), "table_keys": table_keys, "table_rows": table_rows, "per_table_keys_rows": per_table,
            "memos": m.memos.capacity() * 12 + m.memos.values().map(Memo::bytes).sum::<usize>(), "memo_count": m.memos.len(),
            "readers": m.readers.capacity() * 32 + m.readers.values().map(|v| v.capacity() * 4).sum::<usize>(), "reader_keys": m.readers.len(),
            "parents": m.parents.capacity() * 28 + m.parents.values().map(|v| v.capacity() * 4).sum::<usize>(), "parent_keys": m.parents.len(),
            "paths": self.paths.capacity() * 12 + self.paths.values().map(|r| r.as_ref().map_or(0, PathRec::bytes)).sum::<usize>(), "path_count": self.paths.len(),
            "out": self.out.capacity() * 20 + self.out.values().map(|o| o.len() * 8).sum::<usize>(), "out_paths": self.out.len(),
        })
    }

    /// Delta memos by kind: (overriding an entry below, new here, bytes).
    pub fn memo_kinds(&self, qt: &QTable, below: &Below) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        let mut by: FxHashMap<u8, (usize, usize, usize)> = FxHashMap::default();
        for (id, m) in &self.memos.memos {
            let e = by.entry(qt.qs[*id as usize].kind).or_default();
            if below.layers.iter().any(|l| l.memos.memos.contains_key(id)) {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
            e.2 += m.bytes();
        }
        for (k, (o, n, b)) in by {
            out.insert(KIND_NAMES[k as usize].into(), serde_json::json!({"override": o, "new": n, "bytes": b}));
        }
        serde_json::Value::Object(out)
    }

    /// Freeze a delta into a layer. Over nothing (`overlay` false) it is the base: overrides
    /// meaning "not here" are dropped; over layers they are kept as overrides.
    pub fn freeze(self, overlay: bool) -> Base {
        let mut tables = BaseTables::default();
        for (i, m) in self.tables.t.into_iter().enumerate() {
            tables.t[i] = m.into_iter().filter(|(_, r)| overlay || !r.is_empty()).map(|(k, r)| (k, r.into_boxed_slice())).collect();
        }
        Base {
            tables,
            memos: self.memos.freeze(),
            paths: self.paths.into_iter().filter(|(_, r)| overlay || r.is_some()).collect(),
            pkg: self.pkg,
            out: self.out.into_iter().filter(|(_, o)| overlay || !o.is_empty()).collect(),
            counters: self.counters,
            contrib: self.contrib.into_iter().filter(|(_, c)| overlay || *c).collect(),
        }
    }

    /// A worktree at the top of `below`.
    pub fn at(below: &Below) -> Delta {
        match below.layers.first() {
            Some(top) => Delta { pkg: top.pkg.clone(), counters: top.counters, ..Default::default() },
            None => Delta::default(),
        }
    }
}

impl Base {
    /// Heap bytes of the layer (tables, memos with their reverse indexes, paths, edges).
    pub fn total_bytes(&self) -> usize {
        let b = self.bytes();
        b.tables + b.memos + b.paths + b.out + self.contrib.capacity() * 8
    }
    pub fn bytes(&self) -> BaseBytes {
        BaseBytes {
            tables: self.tables.bytes(),
            table_rows: self.tables.rows(),
            memos: self.memos.bytes(),
            memo_count: self.memos.memos.len(),
            paths: self.paths.capacity() * 12 + self.paths.values().map(|r| r.as_ref().map_or(0, PathRec::bytes)).sum::<usize>(),
            out: self.out.capacity() * 20 + self.out.values().map(|o| o.len() * 8).sum::<usize>(),
            edges: self.out.values().map(|o| o.len()).sum(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct BaseBytes {
    pub tables: usize,
    pub table_rows: usize,
    pub memos: usize,
    pub memo_count: usize,
    pub paths: usize,
    pub out: usize,
    pub edges: usize,
}

fn is_init(p: &str) -> Option<String> {
    if p == "__init__.py" {
        Some(String::new())
    } else {
        p.strip_suffix("/__init__.py").map(str::to_owned)
    }
}

fn under(dir: &str, path: &str) -> bool {
    dir.is_empty() || (path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/')
}

/// Apply one block's rows with sign `by`.
fn apply_block(below: &[&BaseTables], dt: &mut DeltaTables, touch: &mut Touch, it: &Interner, p: Sym, b: &Block, by: i32) -> u64 {
    if !b.linked {
        return 0;
    }
    let mut n = 0u64;
    let mut add = |t: u8, k: Sym, row: Row| {
        dt.add(below, touch, t, k, row, by);
        n += 1;
    };
    for &(k, occ, flags, name) in &b.by_name {
        add(T_BYNAME, k, [p, occ, flags, name]);
        add(T_BYNAME_D, name, [p, k, 0, 0]);
    }
    for &(k, t) in &b.aliases {
        add(T_ALIAS, k, [p, t, 0, 0]);
    }
    for &o in &b.opaque {
        add(T_OPAQUE, o, [p, 0, 0, 0]);
    }
    for &m in &b.modules {
        add(T_MODULES, m, [p, 0, 0, 0]);
    }
    for &(c, t, o) in &b.supers {
        add(T_SUPERS, c, [p, t, o, 0]);
        add(T_SUPERS_T, t, [p, c, o, 0]);
        let ts = it.str(t);
        if let Some(i) = ts.find(".<base>.") {
            let short = it.get(&ts[i + 8..]);
            // a short no string spells is in no table yet: intern it (rare)
            let short = short.unwrap_or_else(|| panic!("unInterned short of {ts:?}"));
            add(T_SUPERS_S, short, [p, t, 0, 0]);
        }
    }
    for &c in &b.class_defs {
        let short = it.get(path::last(it.str(c))).unwrap();
        add(T_CLASSSHORT, short, [c, 0, 0, 0]);
    }
    for &m in &b.markers {
        add(T_MARKER, m, [p, 0, 0, 0]);
    }
    for &(_, nm) in &b.edges {
        add(T_EDGESRC, nm, [p, 0, 0, 0]);
    }
    n
}

/// The `<base>` targets a `<imported>`/`<class>` alias key names (base_target's key map).
pub fn base_targets_of_key(k: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in [".<imported>.", ".<class>."] {
        let mut from = 0;
        while let Some(i) = k[from..].find(tok).map(|i| i + from) {
            let t = format!("{}.<base>.{}", &k[..i], &k[i + tok.len()..]);
            if t.find(".<base>.") == Some(i) {
                out.push(t);
            }
            from = i + 1;
        }
    }
    out
}

/// The alias keys whose `<base>` target is `t` (the inverse of `base_targets_of_key`).
fn alias_keys_of_target(t: &str) -> Vec<String> {
    let Some(i) = t.find(".<base>.") else { return Vec::new() };
    [".<imported>.", ".<class>."].iter().map(|tok| format!("{}{tok}{}", &t[..i], &t[i + 8..])).collect()
}

/// `T_ALIAS_T`, aliases by target, holds only the alias keys a `supers` target resolves
/// through (the rows `base_targets` can use): a filtered index over `T_ALIAS` and `T_SUPERS_T`,
/// updated after the blocks of a step from the keys of either that changed.
fn update_alias_index(bt: &[&BaseTables], dt: &mut DeltaTables, touch: &mut Touch, it: &Interner) -> u64 {
    let mut keys: FxHashSet<Sym> = FxHashSet::default();
    {
        let tv = TView { below: bt.to_vec(), delta: dt };
        for ((t, k), before) in &touch.before {
            match *t {
                T_ALIAS => {
                    keys.insert(*k);
                }
                T_SUPERS_T if before.is_empty() != tv.rows(T_SUPERS_T, *k).is_empty() => {
                    for ak in alias_keys_of_target(it.str(*k)) {
                        if let Some(s) = it.get(&ak) {
                            keys.insert(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut edits: Vec<(Sym, Row, i32)> = Vec::new();
    {
        let tv = TView { below: bt.to_vec(), delta: dt };
        let rows_at = |t: u8, k: Sym, before: bool| -> Vec<(Row, u32)> {
            match touch.before.get(&(t, k)).filter(|_| before) {
                Some(r) => r.clone(),
                None => tv.rows(t, k).to_vec(),
            }
        };
        for k in keys {
            let targets = base_targets_of_key(it.str(k));
            let eligible = |before: bool| targets.iter().any(|t| it.get(t).is_some_and(|ts| !rows_at(T_SUPERS_T, ts, before).is_empty()));
            let old = if eligible(true) { rows_at(T_ALIAS, k, true) } else { Vec::new() };
            let new = if eligible(false) { rows_at(T_ALIAS, k, false) } else { Vec::new() };
            for r in old.iter().filter(|r| !new.contains(r)) {
                edits.push((r.0[1], [r.0[0], k, 0, 0], -1));
            }
            for r in new.iter().filter(|r| !old.contains(r)) {
                edits.push((r.0[1], [r.0[0], k, 0, 0], 1));
            }
        }
    }
    let n = edits.len() as u64;
    for (target, row, by) in edits {
        dt.add(bt, touch, T_ALIAS_T, target, row, by);
    }
    n
}

/// Strings `apply_block` looks up must exist: intern the shorts a block's rows key on.
fn intern_shorts(it: &mut Interner, b: &Block) {
    for &(_, t, _) in &b.supers {
        let ts = it.str(t).to_owned();
        if let Some(i) = ts.find(".<base>.") {
            it.intern(&ts[i + 8..]);
        }
    }
    for &c in &b.class_defs {
        let s = path::last(it.str(c)).to_owned();
        it.intern(&s);
    }
}

pub struct Change {
    pub path: Sym,
    pub old: Option<Sym>,
    pub new: Option<Sym>,
}

/// Maintain worktree `d` over the layers `below` for one event's changes. Over no layers and
/// from an empty delta, the same step is the from-scratch build.
pub fn apply(sh: &mut Shared, below: &Below, d: &mut Delta, changes: &[Change]) -> Step {
    let t0 = Instant::now();
    let mut step = Step { paths_changed: changes.len() as u32, ..Default::default() };
    let was_incomplete = d.incomplete();
    let bt = below.tables();
    // 1. new member contents, package markers
    let mut new_content: FxHashMap<Sym, Option<Sym>> = FxHashMap::default();
    let mut dirs_changed: Vec<String> = Vec::new();
    for c in changes {
        let old = d.rec(below, c.path).map(|r| r.content);
        assert_eq!(old, c.old, "{}: old content mismatch", sh.it.str(c.path));
        new_content.insert(c.path, c.new);
        let ps = sh.it.str(c.path).to_owned();
        if let Some(dir) = is_init(&ps) {
            if sh.scope.admits(&ps) {
                let had = c.old.is_some();
                let has = c.new.is_some();
                if had != has {
                    let e = d.pkg.entry(dir.clone()).or_insert(0);
                    if has {
                        *e += 1;
                    } else {
                        *e -= 1;
                    }
                    if *e == 0 {
                        d.pkg.remove(&dir);
                    }
                    dirs_changed.push(dir);
                }
            }
        }
    }
    // 2. affected paths: the changed ones, and (fallback 2) every member under a changed marker
    let mut affected: Vec<Sym> = changes.iter().map(|c| c.path).collect();
    if !dirs_changed.is_empty() {
        let members: Vec<Sym> = d.members(below).into_iter().map(|(p, _)| p).collect();
        for p in members {
            let ps = sh.it.str(p);
            if dirs_changed.iter().any(|dir| under(dir, ps)) && !new_content.contains_key(&p) {
                step.init_fallback = true;
                affected.push(p);
            }
        }
    }
    affected.sort_unstable();
    affected.dedup();
    // 3. per path: old block out, new block in
    let mut touch = Touch::default();
    let mut out_dirty: FxHashSet<Sym> = FxHashSet::default();
    let mut old_recs: FxHashMap<Sym, Option<PathRec>> = FxHashMap::default();
    let pkg = d.pkg.clone();
    let has_pkg = |dir: &str| pkg.contains_key(dir);
    for &p in &affected {
        let ps = sh.it.str(p).to_owned();
        let old = d.rec(below, p).cloned();
        let content = match new_content.get(&p) {
            Some(c) => *c,
            None => old.as_ref().map(|r| r.content),
        };
        let in_scope = sh.scope.admits(&ps);
        let ctx = if in_scope { path::module_ctx(&ps, &has_pkg) } else { ModuleCtx { segments: None, package: false } };
        if let (Some(o), Some(c)) = (&old, content) {
            if o.content == c && o.ctx == ctx {
                continue;
            }
        }
        step.paths_reblocked += 1;
        old_recs.insert(p, old.clone());
        let block_of = |sh: &mut Shared, c: Sym, ctx: &ModuleCtx| -> Block {
            let Shared { sums, it, .. } = sh;
            match sums.get(p, c) {
                Some(sum) => {
                    assert_eq!(sum.in_scope, in_scope, "{ps}: export scope differs from the rule's");
                    path::block(&ps, sum, ctx, it)
                }
                None if in_scope => Block { in_scope: true, refused: Some("no exports for the content".into()), ..Default::default() },
                None => Block::default(),
            }
        };
        if let Some(o) = &old {
            let ob = block_of(sh, o.content, &o.ctx);
            step.table_rows_touched += apply_block(&bt, &mut d.tables, &mut touch, &sh.it, p, &ob, -1);
            if ob.refused.is_some() {
                d.counters.refused_paths -= 1;
            }
            if ob.linked && ob.open {
                d.counters.open_paths -= 1;
            }
        }
        let rec = match content {
            Some(c) => {
                let nb = block_of(sh, c, &ctx);
                intern_shorts(&mut sh.it, &nb);
                step.table_rows_touched += apply_block(&bt, &mut d.tables, &mut touch, &sh.it, p, &nb, 1);
                if nb.refused.is_some() {
                    d.counters.refused_paths += 1;
                }
                if nb.linked && nb.open {
                    d.counters.open_paths += 1;
                }
                Some(PathRec {
                    content: c,
                    ctx,
                    in_scope: nb.in_scope,
                    linked: nb.linked,
                    refused: nb.refused.is_some(),
                    open: nb.open,
                    module: nb.module,
                    defs: nb.defs.into_boxed_slice(),
                    edges: nb.edges.into_boxed_slice(),
                })
            }
            None => None,
        };
        if below.rec(p) == rec.as_ref() {
            d.paths.remove(&p);
        } else {
            d.paths.insert(p, rec);
        }
        out_dirty.insert(p);
    }
    // 3b. the filtered alias index, from the keys the blocks changed
    step.table_rows_touched += update_alias_index(&bt, &mut d.tables, &mut touch, &sh.it);
    // 4. changed keys
    let mut changed: Vec<(u8, Sym)> = Vec::new();
    {
        let tv = TView { below: bt.clone(), delta: &d.tables };
        for ((t, k), before) in &touch.before {
            if tv.rows(*t, *k) != &before[..] {
                changed.push((*t, *k));
            }
        }
    }
    // a touched key whose rows are those below again holds no delta
    for (t, k) in touch.before.keys() {
        let m = &mut d.tables.t[*t as usize];
        let below_r = below_rows(&bt, *t, *k);
        if m.get(k).is_some_and(|r| &r[..] == below_r) {
            m.remove(k);
        }
    }
    changed.sort_unstable();
    step.keys_changed = changed.len() as u64;
    let dep_keys: Vec<u64> = changed.iter().map(|&(t, k)| dep_key(t, sh.it.hash_of(k))).collect();
    // 5. re-run the readers (level order, early cutoff), then roots and refusals
    let mut cx = Ctx::new(TView { below: bt.clone(), delta: &d.tables }, &mut sh.it, &mut sh.qt, &mut sh.defs, below.memos(), &mut d.memos, &mut sh.anomalies);
    cx.propagate(&dep_keys);
    let mut cand: Vec<Q> = Vec::new();
    for &(t, k) in &changed {
        match t {
            T_EDGESRC => cand.push(Q { kind: K_BODY, a: k, b: 0 }),
            T_SUPERS => cand.push(Q { kind: K_REFSUP, a: k, b: 0 }),
            T_BYNAME => cand.push(Q { kind: K_OWNREF, a: k, b: 0 }),
            T_ALIAS => cand.push(Q { kind: K_TYPEKEY, a: k, b: 0 }),
            _ => {}
        }
    }
    let changed_qs = std::mem::take(&mut cx.changed);
    for &id in &changed_qs {
        let qq = cx.qt.qs[id as usize];
        match qq.kind {
            K_BODY | K_REFSUP | K_OWNREF | K_TYPEKEY => cand.push(qq),
            K_LINK if qq.b == 0 => {
                for r in cx.tv.rows(T_EDGESRC, qq.a) {
                    out_dirty.insert(r.0[0]);
                }
            }
            _ => {}
        }
    }
    cand.sort_unstable_by_key(|x| (x.kind, x.a, x.b));
    cand.dedup();
    for qq in cand {
        step.roots_checked += 1;
        let registered = match qq.kind {
            K_BODY => !cx.tv.rows(T_EDGESRC, qq.a).is_empty(),
            K_REFSUP => !cx.tv.rows(T_SUPERS, qq.a).is_empty(),
            K_OWNREF => {
                let rows = cx.tv.rows(T_BYNAME, qq.a);
                let mut names: Vec<u32> = rows.iter().map(|r| r.0[3]).collect();
                names.sort_unstable();
                names.dedup();
                names.len() > 1
            }
            K_TYPEKEY => {
                let ks = cx.it.str(qq.a);
                (ks.contains(".<imported>.") || ks.contains(".<class>.")) && !cx.tv.rows(T_ALIAS, qq.a).is_empty()
            }
            _ => unreachable!(),
        };
        let contributes = if registered {
            if qq.kind == K_BODY {
                cx.get(Q { kind: K_LINK, a: qq.a, b: 0 });
            }
            cx.get(qq).refuses()
        } else {
            false
        };
        // a query never interned never contributed
        let Some(id) = cx.qt.ids.get(&qq).copied() else { continue };
        let before = match d.contrib.get(&id) {
            Some(c) => *c,
            None => below.contrib(id),
        };
        if before != contributes {
            if contributes {
                d.counters.refusal_roots += 1;
            } else {
                d.counters.refusal_roots -= 1;
            }
            if below.contrib(id) == contributes {
                d.contrib.remove(&id);
            } else {
                d.contrib.insert(id, contributes);
            }
        }
    }
    // 6. edges of the paths whose block or whose edge names' links changed
    step.out_paths = out_dirty.len() as u32;
    for p in out_dirty {
        let rec = match d.paths.get(&p) {
            Some(r) => r.as_ref(),
            None => below.rec(p),
        };
        let mut o: Vec<(u32, u32)> = Vec::new();
        if let Some(r) = rec.filter(|r| r.linked) {
            for &(idx, nm) in r.edges.iter() {
                if let V::Link(v) = &*cx.get(Q { kind: K_LINK, a: nm, b: 0 }) {
                    o.extend(v.iter().map(|&dd| (idx, dd)));
                }
            }
        }
        o.sort_unstable();
        o.dedup();
        // the change as (source def, target def) pairs; sources by their (name, occurrence)
        let old_out: &[(u32, u32)] = match d.out.get(&p) {
            Some(x) => x,
            None => below.out(p),
        };
        let old_rec = match old_recs.get(&p) {
            Some(r) => r.as_ref(),
            None => rec,
        };
        let pairs = |defs: &mut DefTable, r: Option<&PathRec>, out: &[(u32, u32)]| -> Vec<(u32, u32)> {
            let Some(r) = r else { return Vec::new() };
            let ids: FxHashMap<u32, u32> = r.defs.iter().zip(path::occurrences(&r.defs)).map(|(x, occ)| (x.idx, defs.id(p, x.name, occ))).collect();
            let mut v: Vec<(u32, u32)> = out.iter().filter_map(|&(idx, dd)| ids.get(&idx).map(|&s| (s, dd))).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let before = pairs(cx.defs, old_rec, old_out);
        let after = pairs(cx.defs, rec, &o);
        step.edges_added.extend(after.iter().filter(|e| before.binary_search(e).is_err()));
        step.edges_removed.extend(before.iter().filter(|e| after.binary_search(e).is_err()));
        step.out_pairs.push((p, before, after));
        let same_as_base = *below.out(p) == *o;
        if same_as_base {
            d.out.remove(&p);
        } else {
            d.out.insert(p, o.into_boxed_slice());
        }
    }
    step.def_paths = old_recs.keys().copied().collect();
    step.old_recs = old_recs;
    step.work = cx.work;
    d.epoch += 1;
    step.incomplete_flip = was_incomplete != d.incomplete();
    step.ms = t0.elapsed().as_secs_f64() * 1e3;
    step
}

/// The worktree's edges as the SQL's `linked_edges` rows (distinct), with Syms for text:
/// (path, start, end, target_path, target_start, target_end, source_name, target_name, class).
pub type EdgeRow = (u32, u32, u32, u32, u32, u32, u32, u32, u32);

/// Every (source def, target def) pair of the worktree, recomputed (checks `Step::edges_*`).
pub fn all_pairs(sh: &mut Shared, below: &Below, d: &Delta) -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    for (p, r) in d.members(below) {
        if !r.linked {
            continue;
        }
        let ids: FxHashMap<u32, u32> = r.defs.iter().zip(path::occurrences(&r.defs)).map(|(x, occ)| (x.idx, sh.defs.id(p, x.name, occ))).collect();
        v.extend(d.out(below, p).iter().filter_map(|&(idx, dd)| ids.get(&idx).map(|&s| (s, dd))));
    }
    v.sort_unstable();
    v.dedup();
    v
}

pub fn edge_rows(sh: &Shared, below: &Below, d: &Delta) -> Vec<EdgeRow> {
    let mut out = Vec::new();
    let find = |r: &PathRec, idx: u32| -> Vec<SDef> { r.defs.iter().filter(|x| x.idx == idx).cloned().collect() };
    let target = |r: &PathRec, name: u32, occ: u32| -> Option<SDef> { path::find_def(&r.defs, name, occ).map(|i| r.defs[i].clone()) };
    for (p, r) in d.members(below) {
        if !r.linked {
            continue;
        }
        for &(idx, dd) in d.out(below, p) {
            let (tp, tname, tocc) = sh.defs.defs[dd as usize];
            let tr = d.rec(below, tp).expect("edge target path is a member");
            for sd in find(r, idx) {
                let td = target(tr, tname, tocc).expect("edge target def is in its path");
                {
                    out.push((p, sd.start, sd.stop, tp, td.start, td.stop, sd.name, td.name, (td.flags & CLASS != 0) as u32));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
