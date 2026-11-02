//! Dead code's reached set as a lattice fact, from facts-engine E3 (`lattice/e3.rs`, "facts-b
//! lattice maint"): the worktree's dead-code product graph (`y2::dc`), kept as a layered CSR over a
//! shared base (`maint::MutLayer`, forward and transposed), its rows re-evaluated at Y2's scope
//! after every edit, and `min_level` (the reached set with exact BFS levels) over it either
//! maintained (E3's levelled re-check, `maint::level_update`) or recomputed from the roots on the
//! current CSR (E1's fixpoint, `round::sequential` or `round::parallel`).
//!
//! The base: worktree 1's product graph (E3's `--base w1`), built cold once (E1's closure: roots,
//! every definition's flow state and its module's), written as columns and mapped (L6). Ids: the
//! base's states in `St` order; a worktree appends local ids for states the base does not hold.
//! Every worktree's row for every id in its space is `succ` under its facts (the invariant `check`
//! verifies), so a fixpoint from its roots is Y2's reachability. Values (levels, seed bits) are the
//! base's columns mapped privately per worktree (`cols::CowCol`): the kernel copies a 16 KB page on
//! a worktree's first write to it.
//!
//! A worktree is the base plus one edit: every base id and its own closure re-evaluated, the rows
//! that differ written to its layers, the values derived from the base's by the same algorithm as
//! an edit.
//!
//! Kept from E3: `Base`, `Ext`, the rows step, `create`, `check`, `memory`. Dropped: `set` (DRed,
//! killed by E3) and `set` by B/F (dead code reads `min_level`), the replay driver (now
//! `deadcode::replay`), the forked check pool (now the replay's), Y3's world (now `Inputs`).
use crate::code_rule_dsl::facts::lattice::build;
use crate::code_rule_dsl::facts::lattice::cols::{self, Col, CowCol};
use crate::code_rule_dsl::facts::lattice::maint::{self, Edit, LayerCounts, LevelCounts, MutLayer, Scratch};
use crate::code_rule_dsl::facts::lattice::round::{self, Csr, Lattice, Rows, Values, INF};
use crate::code_rule_dsl::facts::y2::dc::{self, Dc, Rule, St};
use crate::code_rule_dsl::facts::y2::facts::Facts;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Local ids a worktree may add (address space reserved per value column; untouched pages cost
/// nothing).
const EXT_CAP: usize = 1 << 22;

pub(crate) fn kind(s: St) -> usize {
    match dc::layer(s) {
        dc::NONE => 0,
        dc::ANN => 1,
        _ => 2,
    }
}

/// How the reached set follows the CSR: E3's maintenance, or E1's fixpoint from scratch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// `min_level` from the roots over the current CSR, every time (03 §2: an invalidated fact is
    /// recomputed from its inputs). One thread: `round::sequential`; more: `round::parallel`.
    Recompute { threads: usize },
    /// `min_level` maintained from the edit (E3: levelled re-check, re-derivation, propagation).
    Maintained,
}

/// The shared base: worktree 1's product graph, as mapped columns.
pub(crate) struct Base {
    pub nb: usize,
    pub st: Col<u64>,
    ids: FxHashMap<St, u32>,
    off: Col<u32>,
    tgt: Col<u32>,
    roff: Col<u32>,
    rtgt: Col<u32>,
    lay_key: Col<u32>,
    lay_off: Col<u32>,
    lay_ids: Col<u32>,
    pub lvl: Col<u32>,
    seed: Col<u64>,
    dir: PathBuf,
}

impl Base {
    pub(crate) fn fwd(&self) -> Csr<'_> {
        Csr::trusted(&self.off, &self.tgt)
    }
    pub(crate) fn rev(&self) -> Csr<'_> {
        Csr::trusted(&self.roff, &self.rtgt)
    }
    /// Base ids of node `x`: contiguous (ids are in `St` order, node in the high word).
    fn node_ids(&self, x: u32) -> std::ops::Range<u32> {
        let lo = self.st.partition_point(|&s| s < (x as u64) << 32) as u32;
        let hi = self.st.partition_point(|&s| s < ((x as u64) + 1) << 32) as u32;
        lo..hi
    }
    fn layer_ids(&self, l: u32) -> &[u32] {
        match self.lay_key.binary_search(&l) {
            Ok(i) => &self.lay_ids[self.lay_off[i] as usize..self.lay_off[i + 1] as usize],
            Err(_) => &[],
        }
    }
    /// The base's root states.
    pub(crate) fn seeds(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.nb as u32).filter(|&v| bit(&self.seed, v))
    }

    /// Build the base from `f` (cold, once), write its columns under `dir`, map them back.
    pub(crate) fn build(f: &Facts, dir: &Path) -> (Base, serde_json::Value) {
        std::fs::create_dir_all(dir).unwrap();
        let t = Instant::now();
        let e = build::enumerate(f);
        let enum_ms = t.elapsed().as_secs_f64() * 1e3;
        let mut st: Vec<St> = e.states.clone();
        st.sort_unstable();
        let ids: FxHashMap<St, u32> = st.iter().enumerate().map(|(i, s)| (*s, i as u32)).collect();
        let nb = st.len();
        let mut rows: Vec<Vec<u32>> = vec![Vec::new(); nb];
        for (i, s) in e.states.iter().enumerate() {
            let mut r: Vec<u32> = e.tgt[e.off[i]..e.off[i + 1]].iter().map(|t| ids[t]).collect();
            r.sort_unstable();
            r.dedup();
            rows[ids[s] as usize] = r;
        }
        let mut off = vec![0u32];
        let mut tgt = Vec::new();
        for r in &rows {
            tgt.extend_from_slice(r);
            off.push(tgt.len() as u32);
        }
        drop(rows);
        let csr = Csr::new(&off, &tgt, nb);
        let (roff, rtgt) = round::transpose(&csr);
        let mut lay: Vec<(u32, u32)> = st.iter().enumerate().filter(|(_, s)| dc::layer(**s) >= 2).map(|(i, s)| (dc::layer(*s), i as u32)).collect();
        lay.sort_unstable();
        let (mut lay_key, mut lay_off, mut lay_ids) = (Vec::new(), Vec::new(), Vec::new());
        for (l, i) in &lay {
            if lay_key.last() != Some(l) {
                lay_key.push(*l);
                lay_off.push(lay_ids.len() as u32);
            }
            lay_ids.push(*i);
        }
        lay_off.push(lay_ids.len() as u32);
        let mut seeds: Vec<u32> = e.seeds.iter().map(|s| ids[s]).collect();
        seeds.sort_unstable();
        seeds.dedup();
        let (Values::Levels(lvl), _) = round::sequential(&csr, &seeds, Lattice::MinLevel) else { unreachable!() };
        // The base against Y2 from scratch, state for state and level for level.
        let y2 = Dc::scratch_reach(f).levels();
        assert_eq!(y2.len(), lvl.iter().filter(|l| **l != INF).count(), "base: reached ≠ Y2");
        for (s, l) in &y2 {
            assert_eq!(lvl[ids[s] as usize], *l, "base: level ≠ Y2");
        }
        let mut seed = vec![0u64; nb.div_ceil(64)];
        for &s in &seeds {
            seed[(s >> 6) as usize] |= 1 << (s & 63);
        }
        cols::write(&dir.join("st.u64"), &st);
        cols::write(&dir.join("off.u32"), &off);
        cols::write(&dir.join("tgt.u32"), &tgt);
        cols::write(&dir.join("roff.u32"), &roff);
        cols::write(&dir.join("rtgt.u32"), &rtgt);
        cols::write(&dir.join("lay_key.u32"), &lay_key);
        cols::write(&dir.join("lay_off.u32"), &lay_off);
        cols::write(&dir.join("lay_ids.u32"), &lay_ids);
        cols::write_padded(&dir.join("lvl.u32"), &lvl, INF);
        cols::write_padded(&dir.join("seed.u64"), &seed, 0);
        let kinds = st.iter().fold([0usize; 3], |mut k, s| {
            k[kind(*s)] += 1;
            k
        });
        let info = json!({"ids": nb, "ids_flow_annotated_dispatch": kinds, "edges": tgt.len(), "seeds": seeds.len(), "reached": y2.len(), "enumerate_ms": enum_ms, "ms": t.elapsed().as_secs_f64() * 1e3,
            "bytes": {"csr": (off.len() + tgt.len()) * 4, "transpose": (roff.len() + rtgt.len()) * 4, "st": st.len() * 8, "layer_index": (lay_key.len() + lay_off.len() + lay_ids.len()) * 4, "lvl": lvl.len() * 4, "seed": seed.len() * 8}});
        let b = Base {
            nb,
            st: cols::map(&dir.join("st.u64")),
            ids,
            off: cols::map(&dir.join("off.u32")),
            tgt: cols::map(&dir.join("tgt.u32")),
            roff: cols::map(&dir.join("roff.u32")),
            rtgt: cols::map(&dir.join("rtgt.u32")),
            lay_key: cols::map(&dir.join("lay_key.u32")),
            lay_off: cols::map(&dir.join("lay_off.u32")),
            lay_ids: cols::map(&dir.join("lay_ids.u32")),
            lvl: cols::map(&dir.join("lvl.u32")),
            seed: cols::map(&dir.join("seed.u64")),
            dir: dir.to_path_buf(),
        };
        Csr::new(&b.off, &b.tgt, nb);
        Csr::new(&b.roff, &b.rtgt, nb);
        (b, info)
    }
}

/// A worktree's local ids (states the base does not hold): `st` by id, `sorted` by state (so a
/// node's ids are one range), `layers` (dispatch layer, id) sorted. Ids allocated during a step
/// sit in `pending` until `seal` merges them (one sort per step, not one insertion per id).
#[derive(Default)]
pub(crate) struct Ext {
    nb: usize,
    st: Vec<St>,
    sorted: Vec<(St, u32)>,
    layers: Vec<(u32, u32)>,
    pending: FxHashMap<St, u32>,
}

impl Ext {
    pub(crate) fn n(&self) -> usize {
        self.nb + self.st.len()
    }
    pub(crate) fn st_of(&self, b: &Base, v: u32) -> St {
        if (v as usize) < self.nb {
            b.st[v as usize]
        } else {
            self.st[v as usize - self.nb]
        }
    }
    pub(crate) fn id_of(&self, b: &Base, s: St) -> Option<u32> {
        if let Some(&v) = b.ids.get(&s) {
            return Some(v);
        }
        if let Ok(i) = self.sorted.binary_search_by_key(&s, |x| x.0) {
            return Some(self.sorted[i].1);
        }
        self.pending.get(&s).copied()
    }
    /// `s`'s id, a new local one (queued on `new`) if it has none.
    fn alloc(&mut self, b: &Base, s: St, new: &mut Vec<u32>) -> u32 {
        if let Some(v) = self.id_of(b, s) {
            return v;
        }
        let v = self.n() as u32;
        self.pending.insert(s, v);
        self.st.push(s);
        new.push(v);
        v
    }
    /// Merge the step's new ids into the sorted indexes.
    fn seal(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let mut p: Vec<(St, u32)> = std::mem::take(&mut self.pending).into_iter().collect();
        p.sort_unstable();
        let mut l: Vec<(u32, u32)> = p.iter().filter(|x| dc::layer(x.0) >= 2).map(|x| (dc::layer(x.0), x.1)).collect();
        l.sort_unstable();
        fn merge<T: Ord + Copy>(a: &mut Vec<T>, b: Vec<T>) {
            let mut out = Vec::with_capacity(a.len() + b.len());
            let (mut i, mut j) = (0, 0);
            while i < a.len() || j < b.len() {
                if j == b.len() || (i < a.len() && a[i] < b[j]) {
                    out.push(a[i]);
                    i += 1;
                } else {
                    out.push(b[j]);
                    j += 1;
                }
            }
            *a = out;
        }
        merge(&mut self.sorted, p);
        merge(&mut self.layers, l);
    }
    fn node_ids(&self, x: u32) -> impl Iterator<Item = u32> + '_ {
        let lo = self.sorted.partition_point(|e| e.0 < (x as u64) << 32);
        let hi = self.sorted.partition_point(|e| e.0 < ((x as u64) + 1) << 32);
        self.sorted[lo..hi].iter().map(|e| e.1)
    }
    fn layer_ids(&self, l: u32) -> impl Iterator<Item = u32> + '_ {
        let lo = self.layers.partition_point(|e| e.0 < l);
        let hi = self.layers.partition_point(|e| e.0 <= l);
        self.layers[lo..hi].iter().map(|e| e.1)
    }
    fn bytes(&self) -> usize {
        self.st.capacity() * 8 + self.sorted.capacity() * 16 + self.layers.capacity() * 8 + self.pending.capacity() * 13
    }
}

/// One worktree's lattice state.
pub(crate) struct LWt {
    pub ext: Ext,
    pub fwd: MutLayer,
    pub rev: MutLayer,
    pub lvl: CowCol<u32>,
    pub seed: CowCol<u64>,
}

impl LWt {
    fn empty(b: &Base) -> LWt {
        let words = b.nb.div_ceil(64);
        LWt {
            ext: Ext { nb: b.nb, ..Default::default() },
            fwd: MutLayer::new(b.nb),
            rev: MutLayer::new(b.nb),
            lvl: CowCol::map(&b.dir.join("lvl.u32"), b.nb, b.nb + EXT_CAP),
            seed: CowCol::map(&b.dir.join("seed.u64"), words, (b.nb + EXT_CAP).div_ceil(64)),
        }
    }

    /// The value columns cover every id (new ids unreached until the values step reaches them).
    pub(crate) fn grow(&mut self) {
        let n = self.ext.n();
        self.lvl.grow(n, INF);
    }

    /// Whether state `s` is reached.
    pub(crate) fn reached(&self, b: &Base, s: St) -> bool {
        self.ext.id_of(b, s).is_some_and(|v| self.lvl.as_slice().get(v as usize).is_some_and(|l| *l != INF))
    }
}

/// The CSR side of one step (algebraic counts).
#[derive(Clone, Default, Debug, serde::Serialize)]
pub(crate) struct RowCounts {
    pub dirty_nodes: usize,
    pub dirty_layers: usize,
    /// Ids whose row was re-evaluated (dirty ids, then new ids), by kind (flow, annotated,
    /// dispatch).
    pub recomputed: usize,
    pub recomputed_by_kind: [usize; 3],
    pub succ_targets: usize,
    pub ids_added: usize,
    pub rows_changed: usize,
    pub rows_changed_by_kind: [usize; 3],
    pub edges_del: usize,
    pub edges_ins: usize,
    pub seeds_del: usize,
    pub seeds_ins: usize,
    pub rev_rows_changed: usize,
    pub fwd: LayerCounts,
    pub rev: LayerCounts,
    /// Wall time (not algebraic): `succ` at the recomputed ids; their targets mapped to ids
    /// (a hash probe per target into the base's `St` → id map, a binary search for local ids);
    /// the layers' and seeds' updates (after the rows are known); the whole step.
    pub succ_ms: f64,
    pub map_ms: f64,
    pub layer_ms: f64,
    pub ms: f64,
}

/// The values side of one step.
#[derive(Clone, Default, Debug, serde::Serialize)]
pub(crate) struct ValueCounts {
    /// Ids whose value was derived: the maintenance's invalidated ids, or every id (recompute).
    pub invalidated: u64,
    /// Rounds: invalidation plus propagation rounds (maintained), the fixpoint's rounds
    /// (recompute).
    pub rounds: u64,
    pub changed: u64,
    pub toggled: u64,
    /// `min_level`'s own counts (maintained).
    pub level: Option<LevelCounts>,
    pub ms: f64,
}

/// Reused buffers (per worker).
#[derive(Default)]
pub(crate) struct Tmp {
    sbuf: Vec<St>,
    row: Vec<u32>,
    batch: Vec<(u32, u32, u32)>,
    buf: Vec<u32>,
    pairs: Vec<(u32, u32, bool)>,
    sc: Scratch,
}

#[inline]
fn bit(b: &[u64], v: u32) -> bool {
    ((v >> 6) as usize) < b.len() && b[(v >> 6) as usize] >> (v & 63) & 1 == 1
}

/// The rows of `cand` (ascending, unique) and of every id they add re-evaluated under `f`, the
/// root candidates `seeds` (state, root now) compared with the seed bits; both layers and the
/// seed bits updated. Returns the lattice edit.
fn rows_step(b: &Base, w: &mut LWt, f: &Facts, cand: Vec<u32>, seeds: &[(St, bool)], t: &mut Tmp, c: &mut RowCounts) -> Edit {
    let t0 = Instant::now();
    let LWt { ext, fwd, rev, seed, .. } = w;
    let mut e = Edit::default();
    let mut wl = cand;
    let first_new = ext.n();
    let mut seed_ch: Vec<(u32, bool)> = Vec::new();
    for &(s, now) in seeds {
        let id = if now { Some(ext.alloc(b, s, &mut wl)) } else { ext.id_of(b, s) };
        if let Some(v) = id {
            if bit(seed.as_slice(), v) != now {
                seed_ch.push((v, now));
                if now {
                    e.seeds_ins.push(v);
                } else {
                    e.seeds_del.push(v);
                }
            }
        }
    }
    t.batch.clear();
    t.buf.clear();
    {
        let fr = fwd.rows(b.fwd());
        let nf = fwd.n();
        let mut i = 0;
        while i < wl.len() {
            let v = wl[i];
            i += 1;
            let s = ext.st_of(b, v);
            c.recomputed += 1;
            c.recomputed_by_kind[kind(s)] += 1;
            t.sbuf.clear();
            let ts = Instant::now();
            dc::succ(f, s, &mut t.sbuf);
            let tm = Instant::now();
            c.succ_ms += (tm - ts).as_secs_f64() * 1e3;
            c.succ_targets += t.sbuf.len();
            t.row.clear();
            for &x in &t.sbuf {
                t.row.push(ext.alloc(b, x, &mut wl));
            }
            c.map_ms += tm.elapsed().as_secs_f64() * 1e3;
            t.row.sort_unstable();
            t.row.dedup();
            let cur: &[u32] = if (v as usize) < nf { fr.row(v) } else { &[] };
            if cur == t.row.as_slice() && (v as usize) < nf {
                continue;
            }
            c.rows_changed += 1;
            c.rows_changed_by_kind[kind(s)] += 1;
            // The difference, by merge.
            let (mut a, mut z) = (0, 0);
            while a < cur.len() || z < t.row.len() {
                if z == t.row.len() || (a < cur.len() && cur[a] < t.row[z]) {
                    e.del.push((v, cur[a]));
                    a += 1;
                } else if a == cur.len() || t.row[z] < cur[a] {
                    e.ins.push((v, t.row[z]));
                    z += 1;
                } else {
                    a += 1;
                    z += 1;
                }
            }
            t.batch.push((v, t.buf.len() as u32, t.row.len() as u32));
            t.buf.extend_from_slice(&t.row);
        }
    }
    let tl = Instant::now();
    ext.seal();
    c.ids_added = ext.n() - first_new;
    c.edges_del = e.del.len();
    c.edges_ins = e.ins.len();
    c.seeds_del = e.seeds_del.len();
    c.seeds_ins = e.seeds_ins.len();
    let n = ext.n();
    fwd.grow(n);
    rev.grow(n);
    t.batch.sort_unstable_by_key(|x| x.0);
    c.fwd = fwd.apply(&b.fwd(), &t.batch, &t.buf);
    // The transposed rows of every target an edge left or reached.
    t.pairs.clear();
    t.pairs.extend(e.del.iter().map(|&(s, d)| (d, s, false)));
    t.pairs.extend(e.ins.iter().map(|&(s, d)| (d, s, true)));
    t.pairs.sort_unstable();
    t.batch.clear();
    t.buf.clear();
    {
        let rr = rev.rows(b.rev());
        let mut i = 0;
        while i < t.pairs.len() {
            let d = t.pairs[i].0;
            let mut j = i;
            while j < t.pairs.len() && t.pairs[j].0 == d {
                j += 1;
            }
            let cur = rr.row(d);
            let o = t.buf.len() as u32;
            let (mut a, mut k) = (0, i);
            while a < cur.len() || k < j {
                if k == j || (a < cur.len() && cur[a] < t.pairs[k].1) {
                    t.buf.push(cur[a]);
                    a += 1;
                } else if a == cur.len() || t.pairs[k].1 < cur[a] {
                    debug_assert!(t.pairs[k].2, "removing an absent transposed edge");
                    t.buf.push(t.pairs[k].1);
                    k += 1;
                } else {
                    debug_assert!(!t.pairs[k].2, "adding a present transposed edge");
                    a += 1;
                    k += 1;
                }
            }
            t.batch.push((d, o, t.buf.len() as u32 - o));
            i = j;
        }
    }
    c.rev_rows_changed = t.batch.len();
    c.rev = rev.apply(&b.rev(), &t.batch, &t.buf);
    let words = n.div_ceil(64);
    seed.grow(words, 0);
    let sd = seed.as_mut_slice();
    for (v, now) in seed_ch {
        if now {
            sd[(v >> 6) as usize] |= 1 << (v & 63);
        } else {
            sd[(v >> 6) as usize] &= !(1 << (v & 63));
        }
    }
    c.layer_ms = tl.elapsed().as_secs_f64() * 1e3;
    c.ms = t0.elapsed().as_secs_f64() * 1e3;
    w.grow();
    e
}

/// The rows Y2 scoped (`scope`) re-evaluated under `f` and the roots (`rule`): the lattice edit.
pub(crate) fn rows(b: &Base, w: &mut LWt, f: &Facts, rule: &Rule, scope: &dc::Scope, t: &mut Tmp, c: &mut RowCounts) -> Edit {
    c.dirty_nodes = scope.nodes.len();
    c.dirty_layers = scope.layers.len();
    let mut cand: Vec<u32> = Vec::new();
    for &x in &scope.nodes {
        cand.extend(b.node_ids(x));
        cand.extend(w.ext.node_ids(x));
    }
    for &l in &scope.layers {
        cand.extend_from_slice(b.layer_ids(l));
        cand.extend(w.ext.layer_ids(l));
    }
    cand.sort_unstable();
    cand.dedup();
    let seeds: Vec<(St, bool)> = scope.roots.iter().map(|&x| (dc::st(x, dc::NONE), rule.is_root(f, x))).collect();
    rows_step(b, w, f, cand, &seeds, t, c)
}

/// The reached set after `rows`' edit `e`; appends the ids whose reachability toggled.
pub(crate) fn values(b: &Base, w: &mut LWt, e: &Edit, mode: Mode, t: &mut Tmp, toggled: &mut Vec<u32>) -> ValueCounts {
    let t0 = Instant::now();
    w.grow();
    let LWt { fwd, rev, lvl, seed, ext } = w;
    let fr = fwd.rows(b.fwd());
    let mut r = ValueCounts::default();
    match mode {
        Mode::Maintained => {
            let rr = rev.rows(b.rev());
            let lc = maint::level_update(&fr, &rr, seed.as_slice(), lvl.as_mut_slice(), e, &mut t.sc, toggled);
            r.invalidated = lc.invalidated;
            r.rounds = lc.inval_rounds + lc.prop_rounds;
            r.changed = lc.changed;
            r.toggled = lc.toggled;
            r.level = Some(lc);
        }
        Mode::Recompute { threads } => {
            let n = ext.n();
            let sd = seed.as_slice();
            let seeds: Vec<u32> = (0..n as u32).filter(|&v| bit(sd, v)).collect();
            let (v, counts) = if threads <= 1 { round::sequential(&fr, &seeds, Lattice::MinLevel) } else { round::parallel(&fr, &seeds, Lattice::MinLevel, threads) };
            let Values::Levels(new) = v else { unreachable!() };
            r.invalidated = n as u64;
            r.rounds = counts.rounds.len() as u64;
            // Written only where they differ, so equal pages stay the base's.
            let lv = lvl.as_mut_slice();
            for (i, (old, new)) in lv.iter_mut().zip(&new).enumerate() {
                if *old != *new {
                    r.changed += 1;
                    if (*old == INF) != (*new == INF) {
                        r.toggled += 1;
                        toggled.push(i as u32);
                    }
                    *old = *new;
                }
            }
        }
    }
    r.ms = t0.elapsed().as_secs_f64() * 1e3;
    r
}

/// A worktree with facts `f`: the base plus the edit that turns it into `f` (every base id and
/// `f`'s closure re-evaluated; the roots compared with the base's). Returns its state, the edit
/// from the base and the ids whose reachability differs from the base's (appended to `toggled`).
pub(crate) fn create(b: &Base, f: &Facts, mode: Mode, t: &mut Tmp, toggled: &mut Vec<u32>) -> (LWt, Edit, RowCounts, ValueCounts) {
    let mut w = LWt::empty(b);
    let mut rc = RowCounts::default();
    let mut cand: Vec<u32> = (0..b.nb as u32).collect();
    let mut more = Vec::new();
    for (&id, d) in f.defs.iter() {
        w.ext.alloc(b, dc::st(id, dc::NONE), &mut more);
        w.ext.alloc(b, dc::st(d.module | dc::MOD, dc::NONE), &mut more);
    }
    let mut roots: FxHashSet<St> = Dc::root_states(f).into_iter().collect();
    let mut seeds: Vec<(St, bool)> = Vec::new();
    let sb = w.seed.as_slice();
    for v in 0..b.nb as u32 {
        if bit(sb, v) {
            let s = b.st[v as usize];
            seeds.push((s, roots.remove(&s)));
        }
    }
    seeds.extend(roots.into_iter().map(|s| (s, true)));
    seeds.sort_unstable();
    cand.extend(more);
    let e = rows_step(b, &mut w, f, cand, &seeds, t, &mut rc);
    let vc = values(b, &mut w, &e, mode, t, toggled);
    (w, e, rc, vc)
}

/// Memory of one worktree's lattice state, and its divergence from the base.
pub(crate) fn memory(b: &Base, w: &LWt) -> serde_json::Value {
    let mut by_kind = [0usize; 3];
    for v in w.fwd.replaced_ids() {
        by_kind[kind(w.ext.st_of(b, v))] += 1;
    }
    // Levels that differ from the base's, and the 16 KB pages holding one.
    let l = w.lvl.as_slice();
    let ps = cols::page_size() / 4;
    let (mut diff, mut pages) = (0usize, 0usize);
    for (p, chunk) in l[..b.nb].chunks(ps).enumerate() {
        let base = &b.lvl[p * ps..p * ps + chunk.len()];
        let d = chunk.iter().zip(base).filter(|(a, z)| a != z).count();
        diff += d;
        pages += (d > 0) as usize;
    }
    let reached = l.iter().filter(|x| **x != INF).count();
    json!({
        "ids": w.ext.n(), "ext_ids": w.ext.st.len(), "ext_bytes": w.ext.bytes(),
        "fwd": {"replaced": w.fwd.replaced(), "replaced_flow_annotated_dispatch": by_kind, "live_edges": w.fwd.live_edges(), "bytes": w.fwd.bytes(), "bytes_tight": w.fwd.bytes_tight()},
        "rev": {"replaced": w.rev.replaced(), "live_edges": w.rev.live_edges(), "bytes": w.rev.bytes(), "bytes_tight": w.rev.bytes_tight()},
        "lvl_private": w.lvl.private_bytes(), "seed_private": w.seed.private_bytes(),
        "lvl_differ": diff, "lvl_pages_differ": pages, "lvl_ext": l.len() - b.nb, "reached": reached,
    })
}

/// The maintained worktree against everything from scratch: every row of its id space against
/// `succ` (and every target and root holding an id), no replaced row equal to the base's, the
/// transposed layer against the transpose, the seeds against Y2's roots, the levels against E1's
/// fixpoint on the maintained CSR, and the reached states and levels against Y2's from scratch.
pub(crate) fn check(b: &Base, w: &LWt, f: &Facts) -> Vec<String> {
    let mut diffs: Vec<String> = Vec::new();
    let n = w.ext.n();
    let fr = w.fwd.rows(b.fwd());
    let rr = w.rev.rows(b.rev());
    // Rows = succ, closure complete.
    let (mut bad_rows, mut gaps) = (0usize, 0usize);
    let mut sb = Vec::new();
    let mut row = Vec::new();
    for v in 0..n as u32 {
        sb.clear();
        dc::succ(f, w.ext.st_of(b, v), &mut sb);
        row.clear();
        for s in &sb {
            match w.ext.id_of(b, *s) {
                Some(x) => row.push(x),
                None => gaps += 1,
            }
        }
        row.sort_unstable();
        row.dedup();
        if fr.row(v) != row.as_slice() {
            if bad_rows < 3 {
                diffs.push(format!("row {v} ({:#x}): {:?} maintained / {:?} succ", w.ext.st_of(b, v), fr.row(v), row));
            }
            bad_rows += 1;
        }
    }
    if bad_rows > 0 || gaps > 0 {
        diffs.push(format!("rows: {bad_rows} differ, {gaps} targets without an id"));
    }
    // Layer tight: a base id's replaced row differs from the base's.
    let tight = w.fwd.replaced_ids().into_iter().filter(|&v| (v as usize) < b.nb && fr.row(v) == b.fwd().row(v)).count();
    if tight > 0 {
        diffs.push(format!("layer: {tight} replaced rows equal the base's"));
    }
    // Transpose.
    let (to, tt) = round::transpose(&fr);
    let tc = Csr::new(&to, &tt, n);
    let bad_rev = (0..n as u32).filter(|&v| tc.row(v) != rr.row(v)).count();
    if bad_rev > 0 {
        diffs.push(format!("transpose: {bad_rev} rows differ"));
    }
    // Seeds = Y2's roots.
    let mut roots: Vec<u32> = Vec::new();
    let mut root_gaps = 0;
    for s in Dc::root_states(f) {
        match w.ext.id_of(b, s) {
            Some(v) => roots.push(v),
            None => root_gaps += 1,
        }
    }
    roots.sort_unstable();
    roots.dedup();
    let sd = w.seed.as_slice();
    let seeds: Vec<u32> = (0..n as u32).filter(|&v| bit(sd, v)).collect();
    if seeds != roots || root_gaps > 0 {
        diffs.push(format!("seeds: {} maintained / {} roots ({} without an id)", seeds.len(), roots.len(), root_gaps));
    }
    // E1's fixpoint on the maintained CSR.
    let (Values::Levels(fl), _) = round::sequential(&fr, &roots, Lattice::MinLevel) else { unreachable!() };
    let lv = w.lvl.as_slice();
    if lv.len() != n {
        diffs.push(format!("min_level: {} values for {n} ids", lv.len()));
    }
    let bad_lvl = (0..n.min(lv.len())).filter(|&v| fl[v] != lv[v]).count();
    if bad_lvl > 0 {
        diffs.push(format!("min_level: {bad_lvl} values ≠ the fixpoint on the maintained CSR"));
    }
    // Y2 from scratch: the reached states and their levels.
    let mut reached: FxHashSet<St> = FxHashSet::default();
    for v in 0..n.min(lv.len()) as u32 {
        if lv[v as usize] != INF {
            reached.insert(w.ext.st_of(b, v));
        }
    }
    let y2s = Dc::scratch_reach(f).levels();
    let y2_states: FxHashSet<St> = y2s.iter().map(|x| x.0).collect();
    if y2_states != reached {
        diffs.push(format!("reached: {} lattice / {} Y2 from scratch, {} differ", reached.len(), y2_states.len(), reached.symmetric_difference(&y2_states).count()));
    }
    let bad_y2 = y2s.iter().filter(|(s, l)| w.ext.id_of(b, *s).is_none_or(|v| lv[v as usize] != *l)).count();
    if bad_y2 > 0 {
        diffs.push(format!("levels: {bad_y2} of {} Y2 scratch levels differ", y2s.len()));
    }
    diffs
}
