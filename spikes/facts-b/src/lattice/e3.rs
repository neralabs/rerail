//! `facts-b lattice maint`: experiment E3 (LATTICE.md H7). Y3's wired path (`y3::world`) runs
//! the replay unchanged; after every event, the worktree's dead-code product graph, kept as a
//! layered CSR over a shared base (`maint::MutLayer`, forward and transposed), is updated in
//! place from the edit's scope (Y2's dirty states, `dc::CAPTURE`), and its two lattice facts are
//! maintained in the L5 shape: `min_level` (dead code's reached set with exact BFS levels) and
//! `set` (reached bits, DRed restricted to the reachable part of a deletion).
//!
//! The base: Y3's base layer (strict-majority membership), its product graph built cold once
//! (E1's closure: roots, every definition's flow state and its module's), written as columns and
//! mapped (L6). Ids: the base's states in `St` order; a worktree appends local ids for states the
//! base does not hold. Every worktree's row for every id in its space is `succ` under its facts
//! (the invariant the checks verify), so a fixpoint from its roots is Y2's reachability. Values
//! (levels, set bits, seed bits) are the base's columns mapped privately per worktree
//! (`cols::CowCol`): the kernel copies a 16 KB page on a worktree's first write to it.
//!
//! A worktree (a load, or one created during the replay) is the base plus one edit: every base
//! id and its own closure re-evaluated, the rows that differ written to its layers, the values
//! maintained from the base's by the same algorithms as an edit.
//!
//! Exactness (`--check-every N`, `--check-load`), in a forked child: every row of the worktree's
//! id space against `succ` (and every target and root holding an id), the transposed layer
//! against the transpose, the seeds against Y2's roots, the maintained values against E1's
//! sequential fixpoint on the maintained CSR (both lattices), the reached states against Y2's
//! maintained reached set and the levels against Y2's from-scratch levels.
use super::build;
use super::cols::{self, Col, CowCol};
use super::maint::{self, BfCounts, Edit, LayerCounts, LevelCounts, MutLayer, Scratch, SetCounts};
use super::round::{self, Csr, Lattice, Rows, Values, INF};
use crate::y2::dc::{self, Dc, St};
use crate::y2::facts::Facts;
use crate::y3::world::World;
use crate::y3::{bench, read_events, wire};
use crate::ygraph::replay::{heap, plurality, CLUSTERS};
use crate::ygraph::{read_members, shared as y1_shared};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Local ids a worktree may add (address space reserved per value column; untouched pages cost
/// nothing).
const EXT_CAP: usize = 1 << 22;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn kind(s: St) -> usize {
    match dc::layer(s) {
        dc::NONE => 0,
        dc::ANN => 1,
        _ => 2,
    }
}

/// The shared base: Y3's base layer's product graph, as mapped columns.
pub struct Base {
    nb: usize,
    st: Col<u64>,
    ids: FxHashMap<St, u32>,
    off: Col<u32>,
    tgt: Col<u32>,
    roff: Col<u32>,
    rtgt: Col<u32>,
    lay_key: Col<u32>,
    lay_off: Col<u32>,
    lay_ids: Col<u32>,
    lvl: Col<u32>,
    set: Col<u64>,
    dir: PathBuf,
    /// The base's membership (path → content), for divergence.
    members: FxHashMap<u32, u32>,
}

impl Base {
    fn fwd(&self) -> Csr<'_> {
        Csr::trusted(&self.off, &self.tgt)
    }
    fn rev(&self) -> Csr<'_> {
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
}

/// Build the base from `f` (cold, once), write its columns under `dir`, map them back.
fn build_base(f: &Facts, dir: &Path) -> (Base, serde_json::Value) {
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
    let (Values::Bits(set), _) = round::sequential(&csr, &seeds, Lattice::Set) else { unreachable!() };
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
    cols::write_padded(&dir.join("set.u64"), &set, 0);
    cols::write_padded(&dir.join("seed.u64"), &seed, 0);
    let kinds = st.iter().fold([0usize; 3], |mut k, s| {
        k[kind(*s)] += 1;
        k
    });
    let info = json!({"ids": nb, "ids_flow_annotated_dispatch": kinds, "edges": tgt.len(), "seeds": seeds.len(), "reached": y2.len(), "enumerate_ms": enum_ms, "ms": t.elapsed().as_secs_f64() * 1e3,
        "bytes": {"csr": (off.len() + tgt.len()) * 4, "transpose": (roff.len() + rtgt.len()) * 4, "st": st.len() * 8, "layer_index": (lay_key.len() + lay_off.len() + lay_ids.len()) * 4, "lvl": lvl.len() * 4, "set": set.len() * 8, "seed": seed.len() * 8}});
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
        set: cols::map(&dir.join("set.u64")),
        dir: dir.to_path_buf(),
        members: FxHashMap::default(),
    };
    Csr::new(&b.off, &b.tgt, nb);
    Csr::new(&b.roff, &b.rtgt, nb);
    (b, info)
}

/// A worktree's local ids (states the base does not hold): `st` by id, `sorted` by state (so a
/// node's ids are one range), `layers` (dispatch layer, id) sorted. Ids allocated during a step
/// sit in `pending` until `seal` merges them (one sort per step, not one insertion per id).
#[derive(Default)]
struct Ext {
    nb: usize,
    st: Vec<St>,
    sorted: Vec<(St, u32)>,
    layers: Vec<(u32, u32)>,
    pending: FxHashMap<St, u32>,
}

impl Ext {
    fn n(&self) -> usize {
        self.nb + self.st.len()
    }
    fn st_of(&self, b: &Base, v: u32) -> St {
        if (v as usize) < self.nb {
            b.st[v as usize]
        } else {
            self.st[v as usize - self.nb]
        }
    }
    fn id_of(&self, b: &Base, s: St) -> Option<u32> {
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
pub struct LWt {
    ext: Ext,
    fwd: MutLayer,
    rev: MutLayer,
    lvl: CowCol<u32>,
    set: CowCol<u64>,
    set_bf: CowCol<u64>,
    seed: CowCol<u64>,
}

impl LWt {
    fn empty(b: &Base) -> LWt {
        let words = b.nb.div_ceil(64);
        LWt {
            ext: Ext { nb: b.nb, ..Default::default() },
            fwd: MutLayer::new(b.nb),
            rev: MutLayer::new(b.nb),
            lvl: CowCol::map(&b.dir.join("lvl.u32"), b.nb, b.nb + EXT_CAP),
            set: CowCol::map(&b.dir.join("set.u64"), words, (b.nb + EXT_CAP).div_ceil(64)),
            set_bf: CowCol::map(&b.dir.join("set.u64"), words, (b.nb + EXT_CAP).div_ceil(64)),
            seed: CowCol::map(&b.dir.join("seed.u64"), words, (b.nb + EXT_CAP).div_ceil(64)),
        }
    }
}

/// The CSR side of one step (algebraic counts).
#[derive(Default, Debug, serde::Serialize)]
pub struct RowCounts {
    pub dirty_states: usize,
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
    /// the layers' and seeds' updates (after the rows are known).
    pub succ_ms: f64,
    pub map_ms: f64,
    pub layer_ms: f64,
}

/// Reused buffers (per worker).
#[derive(Default)]
pub struct Tmp {
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
    e
}

/// The lattices maintained after `rows_step`: `min_level`, `set` by DRed, `set` by B/F.
fn values_step(b: &Base, w: &mut LWt, e: &Edit, t: &mut Tmp, r: &mut StepRec) {
    let n = w.ext.n();
    w.lvl.grow(n, INF);
    w.set.grow(n.div_ceil(64), 0);
    w.set_bf.grow(n.div_ceil(64), 0);
    let LWt { fwd, rev, lvl, set, set_bf, seed, .. } = w;
    let fr = fwd.rows(b.fwd());
    let rr = rev.rows(b.rev());
    let t0 = Instant::now();
    r.lvl = maint::level_update(&fr, &rr, seed.as_slice(), lvl.as_mut_slice(), e, &mut t.sc);
    r.lvl_ms = t0.elapsed().as_secs_f64() * 1e3;
    let t1 = Instant::now();
    r.set = maint::set_update(&fr, &rr, seed.as_slice(), set.as_mut_slice(), e, &mut t.sc);
    r.set_ms = t1.elapsed().as_secs_f64() * 1e3;
    let t2 = Instant::now();
    r.set_bf = maint::set_update_bf(&fr, &rr, seed.as_slice(), set_bf.as_mut_slice(), e, &mut t.sc);
    r.set_bf_ms = t2.elapsed().as_secs_f64() * 1e3;
}

/// One step's record.
#[derive(Default, Debug, serde::Serialize)]
pub struct StepRec {
    pub rows_ms: f64,
    pub lvl_ms: f64,
    pub set_ms: f64,
    pub set_bf_ms: f64,
    pub rows: RowCounts,
    pub lvl: LevelCounts,
    pub set: SetCounts,
    pub set_bf: BfCounts,
}

/// The edit Y2 scoped (`scope`), applied to `w` under `f` (and `dc`'s roots).
fn edit(b: &Base, w: &mut LWt, f: &Facts, d: &Dc, scope: &dc::Scope, t: &mut Tmp) -> StepRec {
    let mut r = StepRec::default();
    let t0 = Instant::now();
    let mut nodes: Vec<u32> = scope.dirty.iter().map(|s| dc::node(*s)).collect();
    nodes.sort_unstable();
    nodes.dedup();
    r.rows.dirty_states = scope.dirty.len();
    r.rows.dirty_nodes = nodes.len();
    r.rows.dirty_layers = scope.layers.len();
    let mut cand: Vec<u32> = Vec::new();
    for &x in &nodes {
        cand.extend(b.node_ids(x));
        cand.extend(w.ext.node_ids(x));
    }
    for &l in &scope.layers {
        cand.extend_from_slice(b.layer_ids(l));
        cand.extend(w.ext.layer_ids(l));
    }
    cand.sort_unstable();
    cand.dedup();
    let seeds: Vec<(St, bool)> = scope.roots.iter().map(|&x| (dc::st(x, dc::NONE), d.is_root(f, x))).collect();
    let e = rows_step(b, w, f, cand, &seeds, t, &mut r.rows);
    r.rows_ms = t0.elapsed().as_secs_f64() * 1e3;
    values_step(b, w, &e, t, &mut r);
    r
}

/// A worktree with facts `f`: the base plus the edit that turns it into `f` (every base id and
/// `f`'s closure re-evaluated; the roots compared with the base's).
fn create(b: &Base, f: &Facts, t: &mut Tmp) -> (LWt, StepRec) {
    let mut w = LWt::empty(b);
    let mut r = StepRec::default();
    let t0 = Instant::now();
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
    let e = rows_step(b, &mut w, f, cand, &seeds, t, &mut r.rows);
    r.rows_ms = t0.elapsed().as_secs_f64() * 1e3;
    values_step(b, &mut w, &e, t, &mut r);
    (w, r)
}

/// Memory of one worktree's lattice state, and its divergence from the base.
fn memory(b: &Base, w: &LWt) -> serde_json::Value {
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
    // Reached bits (DRed's array) that differ from the base's, and the words holding one.
    let s = w.set.as_slice();
    let bs = &b.set[..b.nb.div_ceil(64)];
    let set_differ: u32 = s.iter().zip(bs).map(|(a, z)| (a ^ z).count_ones()).sum();
    let set_words_differ = s.iter().zip(bs).filter(|(a, z)| a != z).count();
    let reached = l.iter().filter(|x| **x != INF).count();
    json!({
        "ids": w.ext.n(), "ext_ids": w.ext.st.len(), "ext_bytes": w.ext.bytes(),
        "fwd": {"replaced": w.fwd.replaced(), "replaced_flow_annotated_dispatch": by_kind, "live_edges": w.fwd.live_edges(), "bytes": w.fwd.bytes(), "bytes_tight": w.fwd.bytes_tight()},
        "rev": {"replaced": w.rev.replaced(), "live_edges": w.rev.live_edges(), "bytes": w.rev.bytes(), "bytes_tight": w.rev.bytes_tight()},
        "lvl_private": w.lvl.private_bytes(), "set_private": w.set.private_bytes(), "set_bf_private": w.set_bf.private_bytes(), "seed_private": w.seed.private_bytes(),
        "lvl_differ": diff, "lvl_pages_differ": pages, "lvl_ext": l.len() - b.nb, "reached": reached,
        "set_differ": set_differ, "set_words_differ": set_words_differ,
    })
}

/// The maintained worktree against everything from scratch (module docs).
fn check(b: &Base, w: &LWt, f: &Facts, d: &Dc) -> serde_json::Value {
    let t = Instant::now();
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
    // E1's fixpoint on the maintained CSR, both lattices.
    let (Values::Levels(fl), _) = round::sequential(&fr, &roots, Lattice::MinLevel) else { unreachable!() };
    let (Values::Bits(fs), _) = round::sequential(&fr, &roots, Lattice::Set) else { unreachable!() };
    let lv = w.lvl.as_slice();
    let bad_lvl = (0..n).filter(|&v| fl[v] != lv[v]).count();
    if bad_lvl > 0 {
        diffs.push(format!("min_level: {bad_lvl} values ≠ the fixpoint on the maintained CSR"));
    }
    let bad_set = (0..n as u32).filter(|&v| bit(&fs, v) != bit(w.set.as_slice(), v)).count();
    let bad_bf = (0..n as u32).filter(|&v| bit(&fs, v) != bit(w.set_bf.as_slice(), v)).count();
    if bad_bf > 0 {
        diffs.push(format!("set (B/F): {bad_bf} bits ≠ the fixpoint on the maintained CSR"));
    }
    if bad_set > 0 {
        diffs.push(format!("set: {bad_set} bits ≠ the fixpoint on the maintained CSR"));
    }
    // Y2: the maintained reached set (states), and Y2's levels from scratch.
    let y2m = d.levels();
    let mut reached: FxHashSet<St> = FxHashSet::default();
    for v in 0..n as u32 {
        if lv[v as usize] != INF {
            reached.insert(w.ext.st_of(b, v));
        }
    }
    let y2_states: FxHashSet<St> = y2m.iter().map(|x| x.0).collect();
    if y2_states != reached {
        diffs.push(format!("reached: {} lattice / {} Y2 maintained, {} differ", reached.len(), y2_states.len(), reached.symmetric_difference(&y2_states).count()));
    }
    let y2s = Dc::scratch_reach(f).levels();
    let mut bad_y2 = 0;
    for (s, l) in &y2s {
        if w.ext.id_of(b, *s).is_none_or(|v| lv[v as usize] != *l) {
            bad_y2 += 1;
        }
    }
    if bad_y2 > 0 || y2s.len() != reached.len() {
        diffs.push(format!("levels: {bad_y2} of {} Y2 scratch levels differ", y2s.len()));
    }
    // Y2's maintained levels are an upper witness, not BFS depths: how far they drifted.
    let exact: FxHashMap<St, u32> = y2s.iter().copied().collect();
    let drift = y2m.iter().filter(|(s, l)| exact.get(s) != Some(l)).count();
    json!({"equal": diffs.is_empty(), "differences": diffs, "ids": n, "reached": reached.len(), "y2_levels_drifted": drift, "check_s": t.elapsed().as_secs_f64()})
}

struct Checks {
    file: PathBuf,
    inflight: usize,
    max: usize,
    forked: usize,
}

impl Checks {
    fn reap(&mut self, block: bool) {
        while self.inflight > 0 {
            let mut st = 0;
            let r = unsafe { libc::waitpid(-1, &mut st, if block { 0 } else { libc::WNOHANG }) };
            if r <= 0 {
                break;
            }
            assert!(libc::WIFEXITED(st) && libc::WEXITSTATUS(st) == 0, "check child failed: status {st}");
            self.inflight -= 1;
            if block && self.inflight < self.max {
                break;
            }
        }
    }

    fn spawn(&mut self, b: &Base, world: &World, lw: &BTreeMap<u32, LWt>, w: u32, label: &str) {
        while self.inflight >= self.max {
            self.reap(true);
        }
        std::io::stdout().flush().ok();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let top = &world.wts[&w].top;
                check(b, &lw[&w], &top.facts, top.dc.as_ref().unwrap())
            }));
            let mut line = r.unwrap_or_else(|_| json!({"equal": false, "differences": ["check panicked"]}));
            line["worktree"] = json!(w);
            line["at"] = json!(label);
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file).unwrap();
            f.write_all(format!("{line}\n").as_bytes()).unwrap();
            unsafe { libc::_exit(0) }
        }
        self.inflight += 1;
        self.forked += 1;
        self.reap(false);
    }

    fn finish(&mut self) {
        while self.inflight > 0 {
            self.reap(true);
        }
    }
}

/// Paths whose content differs from the lattice base's membership (`b.members`, fixed when the
/// base was built).
fn divergence(b: &Base, world: &World, w: u32) -> usize {
    let m = &world.wts[&w].top.members;
    let mut d = m.iter().filter(|(p, c)| b.members.get(p) != Some(c)).count();
    d += b.members.keys().filter(|p| !m.contains_key(p)).count();
    d
}

fn sample(b: &Base, world: &World, lw: &BTreeMap<u32, LWt>, at: &str) -> serde_json::Value {
    let per: Vec<serde_json::Value> = lw
        .iter()
        .map(|(w, s)| {
            let mut m = memory(b, s);
            m["worktree"] = json!(w);
            m["divergence"] = json!(divergence(b, world, *w));
            m["y2_dc_bytes"] = json!(world.wts[w].top.dc.as_ref().map_or(0, |d| d.bytes().0));
            m
        })
        .collect();
    let (h, fp) = heap();
    json!({"at": at, "worktrees": per, "heap_mb": h as f64 / 1e6, "footprint_mb": fp as f64 / 1e6, "load_average": cols::load_average()})
}

/// `facts-b lattice maint --out DIR [--limit N] [--check-every N] [--check-load] [--workers W]`.
pub fn run(args: &[String]) {
    let out = PathBuf::from(arg(args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let bb = bench();
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    let check_every: usize = arg(args, "--check-every").map_or(0, |x| x.parse().unwrap());
    let check_load = args.iter().any(|a| a == "--check-load");
    let workers: usize = arg(args, "--workers").map_or(8, |x| x.parse().unwrap());
    let exports = PathBuf::from(arg(args, "--exports").unwrap_or_else(|| bb.join("explore/x6-rules/exports/replay-symbols-merged.jsonl").display().to_string()));
    let spec = PathBuf::from(arg(args, "--spec").unwrap_or_else(|| bb.join("explore/lattice-e3/spec-replay-m2-three.json").display().to_string()));
    // The lattice's base: worktree 1's graph (`w1`: E1's and Y2's base for layering), or Y3's
    // strict-majority base layer (`majority`).
    let base_kind = arg(args, "--base").unwrap_or_else(|| "w1".into());
    assert!(base_kind == "w1" || base_kind == "majority", "--base w1|majority");
    let events_p = PathBuf::from(arg(args, "--events").unwrap_or_else(|| bb.join("spike-b/edits/seed-5eedb/events.jsonl").display().to_string()));
    let mdir = bb.join("spike-b/membership");
    let mut checks = Checks { file: out.join("checks.jsonl"), inflight: 0, max: workers, forked: 0 };
    let _ = std::fs::remove_file(&checks.file);
    let t_all = Instant::now();
    dc::CAPTURE.store(true, std::sync::atomic::Ordering::Relaxed);

    // Y3's world, as `y3 replay` builds it.
    let mut sh = wire::Shared { y1: y1_shared(), sym: Default::default(), y2_path: Default::default() };
    crate::ygraph::summary::load(&exports, &mut sh.y1.it, &mut sh.y1.sums, None);
    crate::y3::input::load(&exports, &mut sh.sym.blocks, None);
    let m2 = crate::y3::m2::M2::new(&spec);
    let base_m = read_members(&mdir.join("base.tsv"));
    let (base, _) = wire::base_layer(&mut sh, &base_m);
    let mut world = World { sh, layers: vec![base], m2, wts: BTreeMap::new(), names: BTreeMap::new() };
    let ms: Vec<Vec<(String, String)>> = (1..=40u32).map(|w| read_members(&mdir.join(format!("{w:02}.tsv")))).collect();
    for &(lo, hi) in &CLUSTERS {
        let cm = plurality(&ms, lo, hi);
        let World { sh, layers, .. } = &mut world;
        let b = crate::y3::world::below(layers, 0);
        let (top, _) = wire::over(sh, &b, &layers[0], &cm);
        let layer = top.freeze(true);
        world.layers.push(layer);
    }
    eprintln!("e3: Y3 world: base + {} overlays in {:.1}s", CLUSTERS.len(), t_all.elapsed().as_secs_f64());

    let mut t = Tmp::default();
    let mut lw: BTreeMap<u32, LWt> = BTreeMap::new();
    let layer_of = |w: u32| CLUSTERS.iter().position(|&(lo, hi)| lo <= w && w <= hi).map_or(0, |c| c + 1);
    let mut loads = Vec::new();
    let mut y3_loads: BTreeMap<u32, f64> = BTreeMap::new();
    let st1 = world.create(1, layer_of(1), &ms[0]);
    y3_loads.insert(1, st1.ms);
    // The lattice base.
    let base_facts = if base_kind == "w1" { &world.wts[&1].top.facts } else { &world.layers[0].top.facts };
    let (mut b, mut base_info) = build_base(base_facts, &out.join("base"));
    let bm = if base_kind == "w1" { &world.wts[&1].top.members } else { &world.layers[0].top.members };
    b.members = bm.iter().map(|(p, c)| (*p, *c)).collect();
    base_info["kind"] = json!(base_kind);
    eprintln!("e3: base {}", base_info);
    for w in 1..=40u32 {
        if w > 1 {
            let st = world.create(w, layer_of(w), &ms[w as usize - 1]);
            y3_loads.insert(w, st.ms);
        }
        let tc = Instant::now();
        let (s, r) = create(&b, &world.wts[&w].top.facts, &mut t);
        let ms_ = tc.elapsed().as_secs_f64() * 1e3;
        lw.insert(w, s);
        loads.push(json!({"worktree": w, "y3_ms": y3_loads[&w], "create_ms": ms_, "step": r, "divergence": divergence(&b, &world, w)}));
        if check_load {
            checks.spawn(&b, &world, &lw, w, &format!("load w{w:02}"));
        }
    }
    let mut samples = vec![sample(&b, &world, &lw, "loaded")];
    eprintln!("e3: 40 worktrees loaded at {:.1}s", t_all.elapsed().as_secs_f64());

    let events = read_events(&events_p);
    let events: Vec<_> = events.into_iter().take(limit.unwrap_or(usize::MAX)).collect();
    let mut per_event = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        let created = ev.group == "new_worktree" || ev.group == "commit_init";
        let mut create_rec = None;
        if created {
            world.create(ev.worktree, 0, &base_m);
            let tc = Instant::now();
            let (s, r) = create(&b, &world.wts[&ev.worktree].top.facts, &mut t);
            create_rec = Some(json!({"ms": tc.elapsed().as_secs_f64() * 1e3, "step": r}));
            lw.insert(ev.worktree, s);
        }
        let mut st = world.apply(ev.worktree, &ev.changes);
        let la = cols::load_average()[0];
        let top = &world.wts[&ev.worktree].top;
        let rec = match st.scope.take() {
            Some(scope) => edit(&b, lw.get_mut(&ev.worktree).unwrap(), &top.facts, top.dc.as_ref().unwrap(), &scope, &mut t),
            None => StepRec::default(),
        };
        let y2 = st.dc.as_ref().map(|d| {
            json!({"dc_ms": d.prepare_ms + d.finish_ms, "prepare_ms": d.prepare_ms, "scope_ms": d.scope_ms, "finish_ms": d.finish_ms, "findings_ms": d.findings_ms,
                   "dirty_states": d.dirty_states, "dirty_reached": d.dirty_reached, "succ_calls": d.succ_calls, "edges_deleted": d.edges_deleted, "edges_inserted": d.edges_inserted,
                   "unsupported": d.over_deleted, "rederived": d.rederived, "propagated": d.propagated, "toggled": d.toggled, "pred_checks": d.pred_checks, "findings_evaluated": d.findings_evaluated})
        });
        let wl = &lw[&ev.worktree];
        per_event.push(json!({
            "i": i, "seq": ev.seq, "worktree": ev.worktree, "group": ev.group, "kind": ev.kind, "paths": st.paths, "load_1m": la,
            "y3_ms": st.ms, "y2": y2, "lat": rec, "create": create_rec,
            "layer": {"fwd_replaced": wl.fwd.replaced(), "fwd_bytes": wl.fwd.bytes(), "rev_replaced": wl.rev.replaced(), "rev_bytes": wl.rev.bytes(), "ext_ids": wl.ext.st.len()},
        }));
        if check_every > 0 && (i + 1) % check_every == 0 {
            checks.spawn(&b, &world, &lw, ev.worktree, &format!("event {}", ev.seq));
        }
        if (i + 1) % 250 == 0 {
            eprintln!("e3: event {} at {:.1}s, checks forked {}", i + 1, t_all.elapsed().as_secs_f64(), checks.forked);
        }
        if (i + 1) % 500 == 0 {
            samples.push(sample(&b, &world, &lw, &format!("event {}", i + 1)));
        }
    }
    checks.finish();
    samples.push(sample(&b, &world, &lw, "end"));
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&checks.file).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let unequal: Vec<&serde_json::Value> = lines.iter().filter(|l| !l["equal"].as_bool().unwrap()).collect();
    let res = json!({
        "base": base_info, "loads": loads, "memory": samples, "events": per_event.len(), "seconds": t_all.elapsed().as_secs_f64(),
        "exactness": {"checks": lines.len(), "forked": checks.forked, "unequal": unequal.len(), "first_unequal": unequal.iter().take(10).collect::<Vec<_>>(), "check_every": check_every, "check_load": check_load},
        "copied_bytes": cols::COPIED.load(std::sync::atomic::Ordering::Relaxed),
    });
    std::fs::write(out.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", json!({"events": per_event.len(), "seconds": res["seconds"], "exactness": res["exactness"]}));
    if !unequal.is_empty() {
        std::process::exit(1);
    }
}
