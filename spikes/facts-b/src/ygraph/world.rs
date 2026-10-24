//! Base and worktree state, and the per-event maintenance step: per-path blocks deleted and
//! reinserted, keyed tables updated with the changed keys collected, the readers of those keys
//! re-run in level order (engine), roots registered, refusals and edges updated.
use super::engine::*;
use super::intern::{Interner, Sym};
use super::path::{self, Block, ModuleCtx, SDef, CLASS};
use super::summary::Summaries;
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Instant;

/// Definition ids: one per (path, def idx), shared by the base and every worktree. A def keeps
/// its id while its path's content changes (spans are read through the worktree's view).
#[derive(Default)]
pub struct DefTable {
    pub map: FxHashMap<(u32, u32), u32>,
    pub defs: Vec<(u32, u32)>,
}

impl DefTable {
    pub fn id(&mut self, path: u32, idx: u32) -> u32 {
        if let Some(&d) = self.map.get(&(path, idx)) {
            return d;
        }
        let d = self.defs.len() as u32;
        self.defs.push((path, idx));
        self.map.insert((path, idx), d);
        d
    }
    pub fn bytes(&self) -> usize {
        self.map.capacity() * 13 + self.defs.capacity() * 8
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

/// The frozen base graph state.
#[derive(Default)]
pub struct Base {
    pub tables: BaseTables,
    pub memos: BaseMemos,
    pub paths: FxHashMap<Sym, PathRec>,
    pub pkg: FxHashMap<String, u32>,
    pub out: FxHashMap<Sym, Box<[(u32, u32)]>>,
    pub counters: Counters,
    pub contrib: FxHashSet<QId>,
}

/// One worktree: the rows that differ from the base.
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
}

impl Delta {
    pub fn rec<'a>(&'a self, base: &'a Base, p: Sym) -> Option<&'a PathRec> {
        match self.paths.get(&p) {
            Some(r) => r.as_ref(),
            None => base.paths.get(&p),
        }
    }

    pub fn out<'a>(&'a self, base: &'a Base, p: Sym) -> &'a [(u32, u32)] {
        match self.out.get(&p) {
            Some(o) => o,
            None => base.out.get(&p).map_or(&[], |o| o),
        }
    }

    pub fn members<'a>(&'a self, base: &'a Base) -> Vec<(Sym, &'a PathRec)> {
        let mut out: Vec<(Sym, &PathRec)> = base.paths.iter().filter(|(p, _)| !self.paths.contains_key(p)).map(|(p, r)| (*p, r)).collect();
        out.extend(self.paths.iter().filter_map(|(p, r)| r.as_ref().map(|r| (*p, r))));
        out
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

    /// Freeze a delta built over the empty base into a base.
    pub fn freeze(self) -> Base {
        let mut tables = BaseTables::default();
        for (i, m) in self.tables.t.into_iter().enumerate() {
            tables.t[i] = m.into_iter().filter(|(_, r)| !r.is_empty()).map(|(k, r)| (k, r.into_boxed_slice())).collect();
        }
        Base {
            tables,
            memos: self.memos.freeze(),
            paths: self.paths.into_iter().filter_map(|(p, r)| r.map(|r| (p, r))).collect(),
            pkg: self.pkg,
            out: self.out.into_iter().filter(|(_, o)| !o.is_empty()).collect(),
            counters: self.counters,
            contrib: self.contrib.into_iter().filter(|(_, c)| *c).map(|(q, _)| q).collect(),
        }
    }

    /// A worktree at the base.
    pub fn at_base(base: &Base) -> Delta {
        Delta { pkg: base.pkg.clone(), counters: base.counters, ..Default::default() }
    }
}

impl Base {
    pub fn bytes(&self) -> BaseBytes {
        BaseBytes {
            tables: self.tables.bytes(),
            table_rows: self.tables.rows(),
            memos: self.memos.bytes(),
            memo_count: self.memos.memos.len(),
            paths: self.paths.capacity() * 12 + self.paths.values().map(PathRec::bytes).sum::<usize>(),
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
fn apply_block(base: &BaseTables, dt: &mut DeltaTables, touch: &mut Touch, it: &Interner, p: Sym, b: &Block, by: i32) -> u64 {
    if !b.linked {
        return 0;
    }
    let mut n = 0u64;
    let mut add = |t: u8, k: Sym, row: Row| {
        dt.add(base, touch, t, k, row, by);
        n += 1;
    };
    for &(k, idx, flags, name) in &b.by_name {
        add(T_BYNAME, k, [p, idx, flags, name]);
        add(T_BYNAME_D, name, [p, k, idx, 0]);
    }
    for &(k, t) in &b.aliases {
        add(T_ALIAS, k, [p, t, 0, 0]);
        add(T_ALIAS_T, t, [p, k, 0, 0]);
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
    for &(idx, nm) in &b.edges {
        add(T_EDGESRC, nm, [p, idx, 0, 0]);
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

/// Maintain worktree `d` over `base` for one event's changes. With an empty base and an empty
/// delta, the same step is the from-scratch build.
pub fn apply(sh: &mut Shared, base: &Base, d: &mut Delta, changes: &[Change]) -> Step {
    let t0 = Instant::now();
    let mut step = Step { paths_changed: changes.len() as u32, ..Default::default() };
    let was_incomplete = d.incomplete();
    // 1. new member contents, package markers
    let mut new_content: FxHashMap<Sym, Option<Sym>> = FxHashMap::default();
    let mut dirs_changed: Vec<String> = Vec::new();
    for c in changes {
        let old = d.rec(base, c.path).map(|r| r.content);
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
        let members: Vec<Sym> = d.members(base).into_iter().map(|(p, _)| p).collect();
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
    let pkg = d.pkg.clone();
    let has_pkg = |dir: &str| pkg.contains_key(dir);
    for &p in &affected {
        let ps = sh.it.str(p).to_owned();
        let old = d.rec(base, p).cloned();
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
            step.table_rows_touched += apply_block(&base.tables, &mut d.tables, &mut touch, &sh.it, p, &ob, -1);
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
                step.table_rows_touched += apply_block(&base.tables, &mut d.tables, &mut touch, &sh.it, p, &nb, 1);
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
        if base.paths.get(&p) == rec.as_ref() {
            d.paths.remove(&p);
        } else {
            d.paths.insert(p, rec);
        }
        out_dirty.insert(p);
    }
    // 4. changed keys
    let mut changed: Vec<(u8, Sym)> = Vec::new();
    {
        let tv = TView { base: &base.tables, delta: &d.tables };
        for ((t, k), before) in &touch.before {
            if tv.rows(*t, *k) != &before[..] {
                changed.push((*t, *k));
            }
        }
    }
    changed.sort_unstable();
    step.keys_changed = changed.len() as u64;
    let dep_keys: Vec<u64> = changed.iter().map(|&(t, k)| dep_key(t, sh.it.hash_of(k))).collect();
    // 5. re-run the readers (level order, early cutoff), then roots and refusals
    let mut cx = Ctx::new(TView { base: &base.tables, delta: &d.tables }, &mut sh.it, &mut sh.qt, &mut sh.defs, &base.memos, &mut d.memos, &mut sh.anomalies);
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
            None => base.contrib.contains(&id),
        };
        if before != contributes {
            if contributes {
                d.counters.refusal_roots += 1;
            } else {
                d.counters.refusal_roots -= 1;
            }
            if base.contrib.contains(&id) == contributes {
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
            None => base.paths.get(&p),
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
        let same_as_base = base.out.get(&p).map_or(o.is_empty(), |b| **b == *o);
        if same_as_base {
            d.out.remove(&p);
        } else {
            d.out.insert(p, o.into_boxed_slice());
        }
    }
    step.work = cx.work;
    d.epoch += 1;
    step.incomplete_flip = was_incomplete != d.incomplete();
    step.ms = t0.elapsed().as_secs_f64() * 1e3;
    step
}

/// The worktree's edges as the SQL's `linked_edges` rows (distinct), with Syms for text:
/// (path, start, end, target_path, target_start, target_end, source_name, target_name, class).
pub type EdgeRow = (u32, u32, u32, u32, u32, u32, u32, u32, u32);

pub fn edge_rows(sh: &Shared, base: &Base, d: &Delta) -> Vec<EdgeRow> {
    let mut out = Vec::new();
    let find = |r: &PathRec, idx: u32| -> Vec<SDef> { r.defs.iter().filter(|x| x.idx == idx).cloned().collect() };
    for (p, r) in d.members(base) {
        if !r.linked {
            continue;
        }
        for &(idx, dd) in d.out(base, p) {
            let (tp, tidx) = sh.defs.defs[dd as usize];
            let tr = d.rec(base, tp).expect("edge target path is a member");
            for sd in find(r, idx) {
                for td in find(tr, tidx) {
                    out.push((p, sd.start, sd.stop, tp, td.start, td.stop, sd.name, td.name, (td.flags & CLASS != 0) as u32));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
