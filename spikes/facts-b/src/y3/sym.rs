//! The symbol table dead code and `exported-families` read (the X6 pack's `proj_*` stages and
//! the per-definition traits), maintained in Y1's style instead of by the dataflow: keyed tables
//! fed by per-content blocks (`input`), and memoised keyed queries that record the table keys
//! and the queries they read; after an edit the readers of the changed keys re-run lowest level
//! first, and a re-run whose value is unchanged stops there. Base → cluster overlay → worktree
//! delta, as Y1's graph (`lmap`).
//!
//! What is per content (no table, no query): a definition candidate's traits and annotation
//! names, the import tokens, base-class rows, `__all__` rows, module names, the probes. What is
//! keyed (one query per key):
//!
//! | query | key | SQL | reads |
//! |---|---|---|---|
//! | `decl` | id | `proj_declarations` (window by id: max `__start`, then min `__path`) | `T_CAND(id)` |
//! | `imp` | (module, name, base IS NULL) | `proj_imports` (window: max ordinal, position, min path) | `T_TOK(key)` |
//! | `res` | fqname | `proj_resolved_names` (the closure through symbol imports, per name) | `T_FQ`, `T_IMPFQ` at each name reached, `imp` |
//! | `bt` | (module, short) | `proj_base_edges`' `coalesce` of three class lookups | `imp`, `T_FQ`, `T_MN`, `T_CLSN`, `decl` |
//! | `pb` | path | `proj_base_edges` rows of a path's `py_bases` | `T_PATH`, `decl`, `bt` |
//! | `be` | class | base edges by owner | `decl`, `pb` |
//! | `hier` | class | `proj_hierarchy` (BFS: depth, then least route) | `decl`, `be` |
//! | `pub` | class | `proj_public_methods` (window by (owner, name)) | `hier`, `decl`, `T_MCLS` |
//! | `roots` | path | `proj_export_names` ⋈ `proj_resolved_names`, `__all__` gate | `T_PATH`, `imp`, `decl`, `res` |
//!
//! The per-definition outputs (`dc__dead_traits`, `ef__traits`, `proj_annotation_names`) are the
//! selected candidate's per-content rows: `decl`'s value. Every query run of an output kind is
//! logged with its value before the event; the rows of the old and new values are the presence
//! changes handed to Y2 (`Change`). Exactness: X5's read-set argument (a query whose recorded
//! reads are unchanged returns the same value), with absent keys recorded as reads too.
use super::input::{Blocks, Cand, SymBlock, Tok, ABSTRACT, DIRECT_LAZY, EXEC_SURFACE, IMPLICIT, LAZY_DECORATED, LEGACY_ASYNC, LEGACY_SYNC, PRIVATE_API, PROPERTY, PROTOCOL, STRUCTURAL};
use crate::lmap::{LMap, Rev};
use crate::y2::facts::{sort_changes, string, sym, with, Change, Kind, Row, Tab, Val};
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::LazyLock;

pub type QId = u32;
type TRow = [u32; 3];
type TKey = (u8, u64);

pub const T_CAND: u8 = 0; // id -> (path, content, candidate)
pub const T_TOK: u8 = 1; // (module, name | base IS NULL << 31) -> (path, content, token)
pub const T_FQ: u8 = 2; // fqname -> (id), one per candidate
pub const T_MN: u8 = 3; // (module, name) -> (id)
pub const T_CLSN: u8 = 4; // class name -> (id), class candidates
pub const T_MCLS: u8 = 5; // (module, class name) -> (id), method candidates
pub const T_IMPFQ: u8 = 6; // `module.name` of a symbol token -> (module, name)
pub const T_PATH: u8 = 7; // path -> (content)
pub const T_MODN: u8 = 8; // (module, name) -> (), `dc__module_names`
pub const TABLE_NAMES: [&str; 9] = ["cand", "tok", "fq", "mn", "clsn", "mcls", "impfq", "path", "modn"];

pub const K_DECL: u8 = 0;
pub const K_IMP: u8 = 1;
pub const K_RES: u8 = 2;
pub const K_BT: u8 = 3;
pub const K_PB: u8 = 4;
pub const K_BE: u8 = 5;
pub const K_HIER: u8 = 6;
pub const K_PUB: u8 = 7;
pub const K_ROOTS: u8 = 8;
pub const NK: usize = 9;
pub const KIND_NAMES: [&str; NK] = ["decl", "imp", "res", "bt", "pb", "be", "hier", "pub", "roots"];

const MODULE_KIND: u32 = 1 << 31;

fn k2(a: u32, b: u32) -> u64 {
    (a as u64) << 32 | b as u64
}

fn dep(t: u8, k: u64) -> u64 {
    k.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (t as u64 + 1).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Q {
    pub kind: u8,
    pub a: u32,
    pub b: u32,
}

impl Q {
    fn level(self) -> u32 {
        match self.kind {
            K_DECL | K_IMP => 1,
            K_RES | K_BT => 2,
            K_PB | K_ROOTS => 3,
            K_BE => 4,
            K_HIER => 5,
            K_PUB => 6,
            _ => unreachable!(),
        }
    }
    fn output(self) -> bool {
        !matches!(self.kind, K_BT | K_BE)
    }
}

/// Query ids, shared by every layer and worktree.
#[derive(Default)]
pub struct QTable {
    ids: FxHashMap<Q, QId>,
    qs: Vec<Q>,
}

impl QTable {
    fn id(&mut self, q: Q) -> QId {
        if let Some(&i) = self.ids.get(&q) {
            return i;
        }
        let i = self.qs.len() as QId;
        self.qs.push(q);
        self.ids.insert(q, i);
        i
    }
    pub fn len(&self) -> usize {
        self.qs.len()
    }
    pub fn bytes(&self) -> usize {
        self.ids.capacity() * (std::mem::size_of::<(Q, QId)>() + 1) + self.qs.capacity() * std::mem::size_of::<Q>()
    }
}

/// Everything shared: the per-content blocks and the query ids.
#[derive(Default)]
pub struct SymShared {
    pub blocks: Blocks,
    pub qt: QTable,
}

/// One layer stack's state (the base, a cluster overlay, or a worktree).
#[derive(Default)]
pub struct SymState {
    tables: LMap<TKey, Vec<(TRow, u32)>>,
    memos: LMap<QId, Box<[u32]>>,
    readers: Rev<u64>,
    parents: Rev<QId>,
    /// Members whose content has no exports (the producer failed, or no retained source): they
    /// contribute no rows, and while one is a member the plans reading the table hold a runtime
    /// refusal.
    pub refused: LMap<u32, u32>,
}

/// Counts of one maintenance step.
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct Work {
    pub paths: u64,
    pub rows_touched: u64,
    pub keys_changed: u64,
    pub dirty_seeds: u64,
    pub executed: u64,
    pub fresh: u64,
    pub rerun: u64,
    pub changed: u64,
    pub probes: u64,
    pub registered: u64,
    pub out_changes: u64,
    /// contents without exports added or removed
    pub refused: u64,
    pub ms: f64,
}

#[derive(Default)]
struct Frame {
    ins: Vec<u64>,
    qs: Vec<QId>,
}

struct Ctx<'a> {
    st: &'a mut SymState,
    qt: &'a mut QTable,
    blocks: &'a Blocks,
    frames: Vec<Frame>,
    work: Work,
    /// Output-kind queries run in this step, with their value before it (None: never computed).
    log: FxHashMap<QId, Option<Box<[u32]>>>,
}

struct Rxs {
    data_layer: Regex,
}

static RX: LazyLock<Rxs> = LazyLock::new(|| Rxs { data_layer: Regex::new(r"^data_layer/[^/]+\.py$").unwrap() });

const PUBLIC_EXCLUDED: [&str; 14] = [
    "__aenter__", "__aexit__", "__enter__", "__exit__", "__init__", "__repr__", "__str__", "authorize_internal_access", "disable_thread_affinity_checks", "get_thread_checks_disabled",
    "set_disable_thread_checks", "validate_legacy_delegates", "versioned_read", "versioned_write",
];

/// SQLite's TEXT order of two symbols.
fn cmp_s(a: u32, b: u32) -> std::cmp::Ordering {
    if a == b {
        return std::cmp::Ordering::Equal;
    }
    with(a, |x| with(b, |y| x.as_bytes().cmp(y.as_bytes())))
}

impl<'a> Ctx<'a> {
    fn rows(&mut self, t: u8, k: u64) -> Vec<(TRow, u32)> {
        self.work.probes += 1;
        if let Some(f) = self.frames.last_mut() {
            f.ins.push(dep(t, k));
        }
        self.st.tables.at(&(t, k)).to_vec()
    }

    fn get(&mut self, q: Q) -> Box<[u32]> {
        let id = self.qt.id(q);
        if let Some(f) = self.frames.last_mut() {
            f.qs.push(id);
        }
        if let Some(m) = self.st.memos.get(&id) {
            return m.clone();
        }
        self.work.fresh += 1;
        self.run(id, q)
    }

    fn run(&mut self, id: QId, q: Q) -> Box<[u32]> {
        self.work.executed += 1;
        if q.output() && !self.log.contains_key(&id) {
            let old = self.st.memos.get(&id).cloned();
            self.log.insert(id, old);
        }
        self.frames.push(Frame::default());
        let v: Box<[u32]> = exec(self, q).into_boxed_slice();
        let mut f = self.frames.pop().unwrap();
        f.ins.sort_unstable();
        f.ins.dedup();
        f.qs.sort_unstable();
        f.qs.dedup();
        for k in f.ins {
            self.st.readers.add(k, id);
        }
        for c in f.qs {
            self.st.parents.add(c, id);
        }
        self.st.memos.insert(id, v.clone());
        v
    }

    /// Re-run the readers of the changed keys, lowest level first, with early cutoff.
    fn propagate(&mut self, keys: &[u64]) {
        let mut heap: BinaryHeap<Reverse<(u32, QId)>> = BinaryHeap::new();
        let mut queued: FxHashSet<QId> = FxHashSet::default();
        let mut rs = Vec::new();
        for k in keys {
            rs.clear();
            self.st.readers.get(k, &mut rs);
            for &q in &rs {
                self.work.dirty_seeds += 1;
                if queued.insert(q) {
                    heap.push(Reverse((self.qt.qs[q as usize].level(), q)));
                }
            }
        }
        while let Some(Reverse((_, id))) = heap.pop() {
            let Some(old) = self.st.memos.get(&id).cloned() else { continue };
            let q = self.qt.qs[id as usize];
            self.work.rerun += 1;
            let new = self.run(id, q);
            if new != old {
                self.work.changed += 1;
                rs.clear();
                self.st.parents.get(&id, &mut rs);
                for &p in &rs {
                    if queued.insert(p) {
                        heap.push(Reverse((self.qt.qs[p as usize].level(), p)));
                    }
                }
            }
        }
    }

    fn block(&self, p: u32, c: u32) -> &'a SymBlock {
        self.blocks.get(p, c).unwrap_or_else(|| panic!("no symbol block for {} @ {}", string(p), string(c)))
    }

    /// The selected declaration of `id`: (path, content, candidate).
    fn decl(&mut self, id: u32) -> Option<(u32, u32, u32)> {
        let v = self.get(Q { kind: K_DECL, a: id, b: 0 });
        (!v.is_empty()).then(|| (v[0], v[1], v[2]))
    }

    fn cand(&self, w: (u32, u32, u32)) -> &'a Cand {
        &self.block(w.0, w.1).cands[w.2 as usize]
    }

    fn is_class(&mut self, id: u32) -> bool {
        self.decl(id).is_some_and(|w| self.cand(w).kind == Kind::Class)
    }

    /// The `imp` winner of (module, name, module kind): (path, content, token).
    fn imp(&mut self, module: u32, name: u32, module_kind: bool) -> Option<(u32, u32, u32)> {
        let v = self.get(Q { kind: K_IMP, a: module, b: name | if module_kind { MODULE_KIND } else { 0 } });
        (!v.is_empty()).then(|| (v[0], v[1], v[2]))
    }

    fn tok(&self, w: (u32, u32, u32)) -> &'a Tok {
        &self.block(w.0, w.1).toks[w.2 as usize]
    }

    fn ids(&mut self, t: u8, k: u64) -> Vec<u32> {
        let mut v: Vec<u32> = self.rows(t, k).into_iter().map(|r| r.0[0]).collect();
        v.dedup();
        v
    }

    /// The member content of `p`.
    fn content(&mut self, p: u32) -> Option<u32> {
        let r = self.rows(T_PATH, p as u64);
        assert!(r.len() <= 1, "{}: two contents", string(p));
        r.first().map(|x| x.0[0])
    }
}

fn min_s(it: impl Iterator<Item = u32>) -> Option<u32> {
    it.min_by(|a, b| cmp_s(*a, *b))
}

/// One query's value (the SQL stage at one key).
fn exec(cx: &mut Ctx, q: Q) -> Vec<u32> {
    match q.kind {
        K_DECL => {
            // window by id: max __start, then min __path
            let mut best: Option<(i64, u32, TRow)> = None;
            for (r, _) in cx.rows(T_CAND, q.a as u64) {
                let st = cx.cand((r[0], r[1], r[2])).start;
                let better = match &best {
                    None => true,
                    Some((bs, bp, _)) => st > *bs || (st == *bs && cmp_s(r[0], *bp).is_lt()),
                };
                if better {
                    best = Some((st, r[0], r));
                }
            }
            best.map_or(Vec::new(), |b| b.2.to_vec())
        }
        K_IMP => {
            // window by (module, name, base IS NULL): max ordinal, max position, min path
            let mut best: Option<(i64, i64, u32, TRow)> = None;
            for (r, _) in cx.rows(T_TOK, k2(q.a, q.b)) {
                let t = cx.tok((r[0], r[1], r[2]));
                let better = match &best {
                    None => true,
                    Some((bo, bp, bpath, _)) => (t.ordinal, t.position) > (*bo, *bp) || ((t.ordinal, t.position) == (*bo, *bp) && cmp_s(r[0], *bpath).is_lt()),
                };
                if better {
                    best = Some((t.ordinal, t.position, r[0], r));
                }
            }
            best.map_or(Vec::new(), |b| b.3.to_vec())
        }
        K_RES => {
            // names(fq, s): the declarations at fq; else, through every symbol import named fq,
            // the names of its target (least fixpoint: a search over names)
            let mut out: Vec<u32> = Vec::new();
            let mut seen: FxHashSet<u32> = FxHashSet::default();
            let mut queue = vec![q.a];
            seen.insert(q.a);
            while let Some(x) = queue.pop() {
                let ids = cx.ids(T_FQ, x as u64);
                if !ids.is_empty() {
                    out.extend(ids);
                    continue;
                }
                for (r, _) in cx.rows(T_IMPFQ, x as u64) {
                    if let Some(w) = cx.imp(r[0], r[1], false) {
                        let t = cx.tok(w).target;
                        if seen.insert(t) {
                            queue.push(t);
                        }
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        }
        K_BT => {
            let (module, short) = (q.a, q.b);
            // (a) a symbol import of `short` in the module whose target is a class
            if let Some(w) = cx.imp(module, short, false) {
                let target = cx.tok(w).target;
                let cs: Vec<u32> = cx.ids(T_FQ, target as u64).into_iter().filter(|c| cx.is_class(*c)).collect();
                if let Some(c) = min_s(cs.into_iter()) {
                    return vec![c];
                }
            }
            // (b) a class of that name in the module
            let cs: Vec<u32> = cx.ids(T_MN, k2(module, short)).into_iter().filter(|c| cx.is_class(*c)).collect();
            if let Some(c) = min_s(cs.into_iter()) {
                return vec![c];
            }
            // (c) the one class of that name anywhere
            let cs: Vec<u32> = cx.ids(T_CLSN, short as u64).into_iter().filter(|c| cx.is_class(*c)).collect();
            if cs.len() == 1 {
                return cs;
            }
            Vec::new()
        }
        K_PB => {
            let p = q.a;
            let Some(c) = cx.content(p) else { return Vec::new() };
            let b = cx.block(p, c);
            let mut out: Vec<[u32; 3]> = Vec::new();
            for br in &b.bases {
                let Some(ci) = b.cands.iter().position(|x| x.start == br.owner) else { continue };
                let owner = b.cands[ci].id;
                if cx.decl(owner) != Some((p, c, ci as u32)) {
                    continue;
                }
                let bt = cx.get(Q { kind: K_BT, a: b.module, b: br.short });
                if let Some(&base) = bt.first() {
                    out.push([owner, base, br.start as u32]);
                }
            }
            out.sort_unstable();
            out.dedup();
            out.concat()
        }
        K_BE => {
            let Some(w) = cx.decl(q.a) else { return Vec::new() };
            let pb = cx.get(Q { kind: K_PB, a: w.0, b: 0 });
            pb.chunks_exact(3).filter(|x| x[0] == q.a).flat_map(|x| [x[1], x[2]]).collect()
        }
        K_HIER => {
            let c = q.a;
            if !cx.is_class(c) {
                return Vec::new();
            }
            // depth by BFS; per base the least route (fixed-width ordinals: least sequence)
            let mut best: FxHashMap<u32, (u32, Vec<u32>)> = FxHashMap::default();
            best.insert(c, (0, Vec::new()));
            let mut frontier = vec![c];
            let mut depth = 0;
            while !frontier.is_empty() {
                depth += 1;
                let mut next: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
                for u in &frontier {
                    let ru = best[u].1.clone();
                    let be = cx.get(Q { kind: K_BE, a: *u, b: 0 });
                    for e in be.chunks_exact(2) {
                        let (b, ord) = (e[0], e[1]);
                        if best.contains_key(&b) {
                            continue;
                        }
                        let mut r = ru.clone();
                        r.push(ord);
                        match next.get_mut(&b) {
                            Some(cur) if *cur <= r => {}
                            Some(cur) => *cur = r,
                            None => {
                                next.insert(b, r);
                            }
                        }
                    }
                }
                frontier = next.keys().copied().collect();
                for (b, r) in next {
                    best.insert(b, (depth, r));
                }
            }
            let mut out: Vec<[u32; 3]> = best
                .into_iter()
                .map(|(b, (d, r))| {
                    let route: String = r.iter().map(|o| format!("{o:012}.")).collect();
                    [b, d, sym(&route)]
                })
                .collect();
            out.sort_unstable();
            out.concat()
        }
        K_PUB => {
            let h = cx.get(Q { kind: K_HIER, a: q.a, b: 0 });
            // per method name: least (depth, route, id)
            let mut best: FxHashMap<u32, (u32, String, String, u32)> = FxHashMap::default();
            for x in h.chunks_exact(3) {
                let (b, depth, route) = (x[0], x[1], string(x[2]));
                let Some(wb) = cx.decl(b) else { continue };
                let (module, cname) = (cx.block(wb.0, wb.1).module, cx.cand(wb).name);
                for m in cx.ids(T_MCLS, k2(module, cname)) {
                    let Some(wm) = cx.decl(m) else { continue };
                    let d = cx.cand(wm);
                    if d.kind != Kind::Method || d.bits & PROPERTY != 0 {
                        continue;
                    }
                    let public = with(d.name, |n| !n.starts_with('_') && !PUBLIC_EXCLUDED.contains(&n));
                    if !public {
                        continue;
                    }
                    let key = (depth, route.clone(), string(m), m);
                    match best.get(&d.name) {
                        Some(cur) if (cur.0, &cur.1, &cur.2) <= (key.0, &key.1, &key.2) => {}
                        _ => {
                            best.insert(d.name, key);
                        }
                    }
                }
            }
            let mut out: Vec<[u32; 2]> = best.into_iter().map(|(n, k)| [n, k.3]).collect();
            out.sort_unstable();
            out.concat()
        }
        K_ROOTS => {
            let p = q.a;
            if !with(p, |s| RX.data_layer.is_match(s)) {
                return Vec::new();
            }
            let Some(c) = cx.content(p) else { return Vec::new() };
            let b = cx.block(p, c);
            let mut names: Vec<(u32, u32)> = Vec::new();
            for (ti, t) in b.toks.iter().enumerate() {
                if t.top.is_some() && !t.module_kind && cx.imp(b.module, t.name, false) == Some((p, c, ti as u32)) {
                    names.push((t.name, t.target));
                }
            }
            for (ci, d) in b.cands.iter().enumerate() {
                if d.class_name.is_none() && cx.decl(d.id) == Some((p, c, ci as u32)) {
                    names.push((d.name, d.fqname));
                }
            }
            let mut out: Vec<[u32; 3]> = Vec::new();
            for (name, target) in names {
                if with(name, |n| n.starts_with('_')) {
                    continue;
                }
                let allowed = match b.explicit {
                    None => true,
                    Some((start, array)) => array && b.all_names.iter().any(|&(n, o)| n == name && o == start),
                };
                if !allowed {
                    continue;
                }
                for &s in cx.get(Q { kind: K_RES, a: target, b: 0 }).iter() {
                    out.push([b.module, name, s]);
                }
            }
            out.sort_unstable();
            out.dedup();
            out.concat()
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------------- rows out

fn vs(s: u32) -> Val {
    Val::S(s)
}
fn vo(s: Option<u32>) -> Val {
    s.map_or(Val::Null, Val::S)
}
fn vi(i: i64) -> Val {
    Val::Int(i)
}
fn voi(i: Option<i64>) -> Val {
    i.map_or(Val::Null, Val::Int)
}
fn vb(c: &Cand, bit: u16) -> Val {
    Val::Int((c.bits & bit != 0) as i64)
}

fn kind_sym(k: Kind) -> u32 {
    sym(match k {
        Kind::Function => "function",
        Kind::Method => "method",
        Kind::Class => "class",
    })
}

/// `proj_declarations`' 13 columns.
fn decl_cols(p: u32, b: &SymBlock, c: &Cand) -> Vec<Val> {
    vec![vs(p), vi(c.start), vi(c.end), vs(c.name), vo(c.class_name), vo(c.ret_ann), voi(c.class_start), voi(c.body_start), vs(b.module), vs(kind_sym(c.kind)), vs(c.id), vs(c.fqname), vi(1)]
}

pub fn dead_row(p: u32, b: &SymBlock, c: &Cand) -> Row {
    let mut v = decl_cols(p, b, c);
    v.extend([vs(c.base_name), vo(c.class_id), vb(c, ABSTRACT), vb(c, PROPERTY), vb(c, IMPLICIT), vb(c, PROTOCOL), vb(c, STRUCTURAL)]);
    v.into_boxed_slice()
}

pub fn ef_row(p: u32, b: &SymBlock, c: &Cand) -> Row {
    let mut v = decl_cols(p, b, c);
    v.extend([vb(c, LEGACY_SYNC), vb(c, LEGACY_ASYNC), vb(c, LAZY_DECORATED), vb(c, PRIVATE_API), vb(c, EXEC_SURFACE), vs(c.params), vb(c, DIRECT_LAZY)]);
    v.into_boxed_slice()
}

/// The rows a query value stands for, by table.
fn value_rows(blocks: &Blocks, q: Q, v: &[u32], out: &mut Vec<(Tab, Row)>) {
    if v.is_empty() {
        return;
    }
    match q.kind {
        K_DECL => {
            let b = blocks.get(v[0], v[1]).unwrap();
            let c = &b.cands[v[2] as usize];
            out.push((Tab::DeadTraits, dead_row(v[0], b, c)));
            out.push((Tab::EfTraits, ef_row(v[0], b, c)));
            for &a in c.ann.iter() {
                out.push((Tab::Ann, Box::new([vs(c.id), vs(a)])));
            }
        }
        K_IMP => {
            let b = blocks.get(v[0], v[1]).unwrap();
            let t = &b.toks[v[2] as usize];
            let kind = sym(if t.module_kind { "module" } else { "symbol" });
            out.push((Tab::Imports, Box::new([vs(v[0]), vi(t.ordinal), voi(t.top), vs(b.module), vs(t.name), vs(t.target), vs(kind)])));
        }
        K_RES => out.extend(v.iter().map(|&s| (Tab::Resolved, Box::new([vs(q.a), vs(s)]) as Row))),
        K_PB => out.extend(v.chunks_exact(3).map(|x| (Tab::Bases, Box::new([vs(x[0]), vs(x[1]), vi(x[2] as i64)]) as Row))),
        K_HIER => out.extend(v.chunks_exact(3).map(|x| (Tab::Hier, Box::new([vs(q.a), vs(x[0]), vi(x[1] as i64), vs(x[2])]) as Row))),
        K_PUB => out.extend(v.chunks_exact(2).map(|x| (Tab::Public, Box::new([vs(q.a), vs(x[0]), vs(x[1])]) as Row))),
        K_ROOTS => out.extend(v.chunks_exact(3).map(|x| (Tab::Roots, Box::new([vs(q.a), vs(x[0]), vs(x[1]), vs(x[2])]) as Row))),
        _ => unreachable!(),
    }
}

/// A block's per-content rows that go straight to Y2 (no window, no join): the probes.
fn content_rows(p: u32, b: &SymBlock, out: &mut Vec<(Tab, Row)>) {
    for &(o, i, g) in &b.probe {
        out.push((Tab::Probe, Box::new([vs(p), vi(o), vi(o), vi(i as i64), vi(g as i64)])));
    }
    for &fq in &b.dyn_fqs {
        out.push((Tab::Dyn, Box::new([vs(p), vi(0), vi(0), vs(fq)])));
    }
}

/// `old` out, `new` in, as presence changes (a row in both is no change).
fn diff_rows(old: Vec<(Tab, Row)>, new: Vec<(Tab, Row)>, out: &mut Vec<Change>) {
    let mut o: Vec<(Tab, Row)> = old;
    let mut n: Vec<(Tab, Row)> = new;
    o.sort();
    n.sort();
    let (mut i, mut j) = (0, 0);
    while i < o.len() || j < n.len() {
        if j == n.len() || (i < o.len() && o[i] < n[j]) {
            out.push(Change { tab: o[i].0, row: o[i].1.clone(), added: false });
            i += 1;
        } else if i == o.len() || n[j] < o[i] {
            out.push(Change { tab: n[j].0, row: n[j].1.clone(), added: true });
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
}

// ---------------------------------------------------------------------------------- the step

/// A block's table rows, with sign `by`.
fn block_rows(p: u32, c: u32, b: &SymBlock, by: i32, mut add: impl FnMut(u8, u64, TRow, i32)) {
    add(T_PATH, p as u64, [c, 0, 0], by);
    for (ci, d) in b.cands.iter().enumerate() {
        add(T_CAND, d.id as u64, [p, c, ci as u32], by);
        add(T_FQ, d.fqname as u64, [d.id, 0, 0], by);
        add(T_MN, k2(b.module, d.name), [d.id, 0, 0], by);
        match d.kind {
            Kind::Class => add(T_CLSN, d.name as u64, [d.id, 0, 0], by),
            Kind::Method => add(T_MCLS, k2(b.module, d.class_name.unwrap()), [d.id, 0, 0], by),
            Kind::Function => {}
        }
    }
    for (ti, t) in b.toks.iter().enumerate() {
        add(T_TOK, k2(b.module, t.name | if t.module_kind { MODULE_KIND } else { 0 }), [p, c, ti as u32], by);
        if !t.module_kind {
            add(T_IMPFQ, t.fq as u64, [b.module, t.name, 0], by);
        }
    }
    for &n in &b.module_names {
        add(T_MODN, k2(b.module, n), [0, 0, 0], by);
    }
}

pub struct PathChange {
    pub path: u32,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

impl SymState {
    /// A worktree (or overlay) over this frozen state.
    pub fn view(&self) -> SymState {
        SymState { tables: self.tables.view(), memos: self.memos.view(), readers: self.readers.view(), parents: self.parents.view(), refused: self.refused.view() }
    }

    pub fn freeze(&mut self) {
        self.tables.freeze();
        self.memos.freeze();
        self.readers.freeze();
        self.parents.freeze();
        self.refused.freeze();
    }

    /// (tables, memos, readers, parents) bytes of the top and of the top-most frozen layer.
    pub fn bytes(&self) -> serde_json::Value {
        serde_json::json!({
            "top": {"tables": self.tables.top_bytes(), "memos": self.memos.top_bytes(), "readers": self.readers.top_bytes(), "parents": self.parents.top_bytes(),
                    "table_keys": self.tables.top_len(), "memo_count": self.memos.top_len(), "reader_entries": self.readers.top_len(), "parent_entries": self.parents.top_len()},
            "layer0": {"tables": self.tables.layer_bytes(0), "memos": self.memos.layer_bytes(0), "readers": self.readers.layer_bytes(0), "parents": self.parents.layer_bytes(0)},
        })
    }

    pub fn top_bytes(&self) -> usize {
        self.tables.top_bytes() + self.memos.top_bytes() + self.readers.top_bytes() + self.parents.top_bytes()
    }

    pub fn layer_bytes(&self) -> usize {
        self.tables.layer_bytes(0) + self.memos.layer_bytes(0) + self.readers.layer_bytes(0) + self.parents.layer_bytes(0)
    }

    /// Bytes and entries by table and by query kind, of the top or (`layer`) the top-most frozen
    /// layer; the reverse indexes as one figure each.
    pub fn by_kind(&self, qt: &QTable, layer: bool) -> serde_json::Value {
        let mut tables = vec![(0usize, 0usize); TABLE_NAMES.len()];
        let mut memos = vec![(0usize, 0usize); NK];
        let tb = |(k, v): (&TKey, Option<&Vec<(TRow, u32)>>), out: &mut Vec<(usize, usize)>| {
            let e = &mut out[k.0 as usize];
            e.0 += std::mem::size_of::<(TKey, Option<Vec<(TRow, u32)>>)>() + v.map_or(0, |v| v.capacity() * 16);
            e.1 += 1;
        };
        let mb = |(id, v): (&QId, Option<&Box<[u32]>>), out: &mut Vec<(usize, usize)>| {
            let e = &mut out[qt.qs[*id as usize].kind as usize];
            e.0 += std::mem::size_of::<(QId, Option<Box<[u32]>>)>() + v.map_or(0, |v| v.len() * 4);
            e.1 += 1;
        };
        if layer {
            if let Some(l) = self.tables.layers().first() {
                l.iter().for_each(|(k, v)| tb((k, v.as_ref()), &mut tables));
            }
            if let Some(l) = self.memos.layers().first() {
                l.iter().for_each(|(k, v)| mb((k, v.as_ref()), &mut memos));
            }
        } else {
            self.tables.top_iter().for_each(|(k, v)| tb((k, v.as_ref()), &mut tables));
            self.memos.top_iter().for_each(|(k, v)| mb((k, v.as_ref()), &mut memos));
        }
        let (r, p) = if layer { (self.readers.layer_bytes(0), self.parents.layer_bytes(0)) } else { (self.readers.top_bytes(), self.parents.top_bytes()) };
        serde_json::json!({
            "tables": TABLE_NAMES.iter().zip(tables).map(|(n, v)| (n.to_string(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>(),
            "memos": KIND_NAMES.iter().zip(memos).map(|(n, v)| (n.to_string(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>(),
            "readers": r, "parents": p,
        })
    }

    /// Maintain this state for one event's path changes; the presence changes of Y2's tables
    /// (every table but `Refs`), removals first.
    pub fn apply(&mut self, sh: &mut SymShared, changes: &[PathChange]) -> (Vec<Change>, Work) {
        let t0 = std::time::Instant::now();
        let blocks = &sh.blocks;
        let mut out: Vec<Change> = Vec::new();
        let mut work = Work { paths: changes.len() as u64, ..Default::default() };
        // 1. blocks out and in; the before-rows of every touched key
        let mut before: FxHashMap<TKey, Vec<(TRow, u32)>> = FxHashMap::default();
        let mut direct_old = Vec::new();
        let mut direct_new = Vec::new();
        for ch in changes {
            let old = self.tables.at(&(T_PATH, ch.path as u64)).first().map(|r| r.0[0]).or_else(|| self.refused.get(&ch.path).copied());
            assert_eq!(old, ch.old, "{}: old content", string(ch.path));
            for (c, by) in [(ch.old, -1), (ch.new, 1)] {
                let Some(c) = c else { continue };
                let Some(b) = blocks.get(ch.path, c) else {
                    work.refused += 1;
                    if by < 0 {
                        self.refused.remove(&ch.path);
                    } else {
                        self.refused.insert(ch.path, c);
                    }
                    continue;
                };
                content_rows(ch.path, b, if by < 0 { &mut direct_old } else { &mut direct_new });
                let tables = &mut self.tables;
                block_rows(ch.path, c, b, by, |t, k, row, by| {
                    work.rows_touched += 1;
                    before.entry((t, k)).or_insert_with(|| tables.at(&(t, k)).to_vec());
                    tables.update((t, k), |rows| match rows.binary_search_by(|x| x.0.cmp(&row)) {
                        Ok(i) => {
                            let n = rows[i].1 as i64 + by as i64;
                            assert!(n >= 0, "table {} key {k}: negative count", TABLE_NAMES[t as usize]);
                            if n == 0 {
                                rows.remove(i);
                            } else {
                                rows[i].1 = n as u32;
                            }
                        }
                        Err(i) => {
                            assert!(by > 0, "table {} key {k}: removing an absent row", TABLE_NAMES[t as usize]);
                            rows.insert(i, (row, by as u32));
                        }
                    });
                });
            }
        }
        diff_rows(direct_old, direct_new, &mut out);
        // 2. changed keys
        let mut changed: Vec<TKey> = before.into_iter().filter(|(k, b)| self.tables.at(k) != &b[..]).map(|(k, b)| {
            if k.0 == T_MODN && b.is_empty() != self.tables.at(&k).is_empty() {
                let (m, n) = ((k.1 >> 32) as u32, k.1 as u32);
                out.push(Change { tab: Tab::ModuleNames, row: Box::new([vs(m), vs(n)]), added: b.is_empty() });
            }
            k
        }).collect();
        changed.sort_unstable();
        work.keys_changed = changed.len() as u64;
        let keys: Vec<u64> = changed.iter().map(|&(t, k)| dep(t, k)).collect();
        // 3. readers, then the queries every changed key registers
        let mut cx = Ctx { st: self, qt: &mut sh.qt, blocks, frames: Vec::new(), work: Work::default(), log: FxHashMap::default() };
        cx.propagate(&keys);
        for &(t, k) in &changed {
            let q = match t {
                T_CAND => Some(Q { kind: K_DECL, a: k as u32, b: 0 }),
                T_TOK => Some(Q { kind: K_IMP, a: (k >> 32) as u32, b: k as u32 }),
                T_FQ | T_IMPFQ => Some(Q { kind: K_RES, a: k as u32, b: 0 }),
                _ => None,
            };
            if let Some(q) = q {
                cx.work.registered += 1;
                cx.get(q);
            }
            if t == T_PATH {
                for kind in [K_PB, K_ROOTS] {
                    cx.work.registered += 1;
                    cx.get(Q { kind, a: k as u32, b: 0 });
                }
            }
        }
        // a class declaration registers its hierarchy and public methods (and those may compute
        // further declarations: until none is new)
        let mut done: FxHashSet<QId> = FxHashSet::default();
        loop {
            let decls: Vec<QId> = cx.log.keys().copied().filter(|id| cx.qt.qs[*id as usize].kind == K_DECL && !done.contains(id)).collect();
            if decls.is_empty() {
                break;
            }
            for id in decls {
                done.insert(id);
                // a declaration that stopped being a class is re-run through its reads
                let v = cx.st.memos.get(&id).cloned().unwrap_or_default();
                if v.is_empty() || cx.cand((v[0], v[1], v[2])).kind != Kind::Class {
                    continue;
                }
                let c = cx.qt.qs[id as usize].a;
                for kind in [K_HIER, K_PUB] {
                    cx.work.registered += 1;
                    cx.get(Q { kind, a: c, b: 0 });
                }
            }
        }
        // 4. rows out: every logged output query's old and new value
        let mut work2 = std::mem::take(&mut cx.work);
        let log = std::mem::take(&mut cx.log);
        drop(cx);
        let qt = &sh.qt;
        for (id, old) in log {
            let q = qt.qs[id as usize];
            let new = self.memos.get(&id).cloned().unwrap_or_default();
            let old = old.unwrap_or_default();
            if old == new {
                continue;
            }
            let (mut ro, mut rn) = (Vec::new(), Vec::new());
            value_rows(blocks, q, &old, &mut ro);
            value_rows(blocks, q, &new, &mut rn);
            diff_rows(ro, rn, &mut out);
        }
        sort_changes(&mut out);
        work2.paths = work.paths;
        work2.rows_touched = work.rows_touched;
        work2.keys_changed = work.keys_changed;
        work2.refused = work.refused;
        work2.out_changes = out.len() as u64;
        work2.ms = t0.elapsed().as_secs_f64() * 1e3;
        (out, work2)
    }

    /// Every row of Y2's tables (except `Refs`) this state holds: the output queries' values
    /// over the registered keys, plus the per-content rows (checks and from-scratch loads).
    pub fn all_rows(&self, sh: &SymShared) -> Vec<(Tab, Row)> {
        let mut out = Vec::new();
        for (id, v) in self.memos.iter() {
            let q = sh.qt.qs[*id as usize];
            if q.output() {
                value_rows(&sh.blocks, q, v, &mut out);
            }
        }
        for ((t, k), rows) in self.tables.iter() {
            match *t {
                T_PATH => {
                    for r in rows {
                        content_rows(*k as u32, sh.blocks.get(*k as u32, r.0[0]).unwrap(), &mut out);
                    }
                }
                T_MODN => out.push((Tab::ModuleNames, Box::new([vs((*k >> 32) as u32), vs(*k as u32)]))),
                _ => {}
            }
        }
        out
    }
}
