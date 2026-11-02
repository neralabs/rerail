//! Experiment E3 (LATTICE.md L9, L10): a lattice fact maintained per edit, in the L5 shape.
//!
//! - `MutLayer` / `LRows`: a worktree's edge relation as E1's layered CSR (a frozen base CSR, a
//!   bit per id, replacement rows), updated in place by a batch of new rows. E1's dense rank per
//!   word becomes a per-word *block offset*: the slots (arena offset, length) of a word's set
//!   bits sit contiguously at `wbase[w]`, with slack up to the next power of two, so a bit that
//!   flips shifts at most 63 slots, or moves its word's block to the end of the slot table. The
//!   read is unchanged: one bit test, then the base row or `slot[wbase[w] + popcount(lower
//!   bits)]`. (A dense rank must be rebuilt, with every slot after the flip, on each flip:
//!   O(replaced rows) per edit.) Rows live in an append-only arena, overwritten in place when
//!   they do not grow; slot table and arena are compacted when half dead. Ids past the base
//!   (`nb ≤ v`) are local to the worktree and always have their bit set. The worktree's state
//!   is O(n / 64) words + its replaced rows.
//! - `level_update`: `min_level` maintenance by levelled re-check, as bulk rounds over the CSR
//!   and its transpose: (1) invalidation, level by level: a node at level ℓ keeps its value while
//!   a predecessor at ℓ − 1 that is not invalidated has an edge to it (level 0: while it is a
//!   seed); an invalidated node's successors at ℓ + 1 are re-checked next round; (2) the
//!   invalidated ids re-derive their candidate level from their surviving predecessors (one
//!   pull over their transposed rows); (3) every decrease (re-derived ids, inserted edges, new
//!   seeds) propagates as rounds of uniform level (Dial's buckets = E1's rounds). The levels are
//!   exact BFS depths after every edit (the lattice's value), not Y2's upper witness.
//! - `set_update`: `set` maintenance by over-delete and re-derive (DRed) restricted to the
//!   reached ids that a deleted edge or seed can reach through the new rows; re-derived from the
//!   transposed rows, then forward rounds. Support counts are not used: the product graph has
//!   cycles, where counts never fall to zero.
//! - `set_update_bf`: `set` maintenance by backward/forward checks (Motik et al.'s B/F, for
//!   reachability): each candidate (a deleted edge's target, a deleted seed, a dead id's
//!   successor) is checked by a backward search over the new transposed rows, in frontier
//!   rounds, that stops at the first seed or id already proven this edit. A search that finds
//!   none has visited the candidate's whole backward cone, all of it unreachable: the cone dies
//!   at once and its successors become candidates. No witness is stored: the lattice's state
//!   stays one bit per id.
//!
//! Every phase is a frontier list or bitset scanned against CSR rows; no hash map, no allocation
//! per row (the scratch is per worker and cleared sparsely), no per-key memo.
use super::round::{Csr, Rows, INF};

/// A worktree's mutable replacement rows over a base CSR of `nb` rows.
pub struct MutLayer {
    nb: usize,
    n: usize,
    bits: Vec<u64>,
    /// Per word: where its block of slots starts in `slot`, and the block's capacity.
    wbase: Vec<u32>,
    wcap: Vec<u8>,
    /// (arena offset, length): a word's set bits in id order, then the block's slack.
    slot: Vec<[u32; 2]>,
    live: usize,
    /// Slots of moved blocks.
    dead_slots: usize,
    arena: Vec<u32>,
    /// Arena words no slot points at.
    dead: usize,
}

/// One batch's work on a layer (algebraic).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LayerCounts {
    pub rows_written: u64,
    pub rows_in_place: u64,
    pub rows_appended: u64,
    pub bits_set: u64,
    pub bits_cleared: u64,
    /// Slots shifted inside a word's block, and blocks moved to the end of the table.
    pub slots_shifted: u64,
    pub blocks_moved: u64,
    pub compactions: u64,
}

impl MutLayer {
    pub fn new(nb: usize) -> MutLayer {
        let w = nb.div_ceil(64);
        MutLayer { nb, n: nb, bits: vec![0; w], wbase: vec![0; w], wcap: vec![0; w], slot: Vec::new(), live: 0, dead_slots: 0, arena: Vec::new(), dead: 0 }
    }

    pub fn n(&self) -> usize {
        self.n
    }

    pub fn replaced(&self) -> usize {
        self.live
    }

    /// Live replacement edges.
    pub fn live_edges(&self) -> usize {
        self.arena.len() - self.dead
    }

    /// Bytes held: bits, block offsets and capacities, slots, arena (capacities).
    pub fn bytes(&self) -> usize {
        self.bits.capacity() * 8 + self.wbase.capacity() * 4 + self.wcap.capacity() + self.slot.capacity() * 8 + self.arena.capacity() * 4
    }

    /// Bytes the same rows would hold compacted without slack.
    pub fn bytes_tight(&self) -> usize {
        self.bits.len() * 13 + self.live * 8 + self.live_edges() * 4
    }

    #[inline(always)]
    fn bit(&self, v: u32) -> bool {
        self.bits[(v >> 6) as usize] >> (v & 63) & 1 == 1
    }

    #[inline(always)]
    fn slot_of(&self, v: u32) -> usize {
        let w = (v >> 6) as usize;
        (self.wbase[w] + (self.bits[w] & ((1u64 << (v & 63)) - 1)).count_ones()) as usize
    }

    /// Set `v`'s bit with slot `s`: shift the word's later slots up by one, moving the block to
    /// the end of the table (capacity doubled, at most 64) when full.
    fn insert_bit(&mut self, v: u32, s: [u32; 2], c: &mut LayerCounts) {
        let w = (v >> 6) as usize;
        let b = self.bits[w];
        let cnt = b.count_ones() as usize;
        if cnt == self.wcap[w] as usize {
            let cap = (cnt + 1).next_power_of_two().min(64);
            let nb = self.slot.len();
            let ob = self.wbase[w] as usize;
            self.slot.extend_from_within(ob..ob + cnt);
            self.slot.resize(nb + cap, [0, 0]);
            self.dead_slots += self.wcap[w] as usize;
            self.wbase[w] = nb as u32;
            self.wcap[w] = cap as u8;
            c.blocks_moved += 1;
        }
        let base = self.wbase[w] as usize;
        let p = (b & ((1u64 << (v & 63)) - 1)).count_ones() as usize;
        self.slot.copy_within(base + p..base + cnt, base + p + 1);
        c.slots_shifted += (cnt - p) as u64;
        self.slot[base + p] = s;
        self.bits[w] = b | 1 << (v & 63);
        self.live += 1;
        c.bits_set += 1;
    }

    fn clear_bit(&mut self, v: u32, c: &mut LayerCounts) {
        let w = (v >> 6) as usize;
        let b = self.bits[w];
        let cnt = b.count_ones() as usize;
        let base = self.wbase[w] as usize;
        let p = (b & ((1u64 << (v & 63)) - 1)).count_ones() as usize;
        self.dead += self.slot[base + p][1] as usize;
        self.slot.copy_within(base + p + 1..base + cnt, base + p);
        c.slots_shifted += (cnt - p - 1) as u64;
        self.bits[w] = b & !(1 << (v & 63));
        self.live -= 1;
        c.bits_cleared += 1;
    }

    /// Ids `n..n_new` become local ids, each with an empty replaced row.
    pub fn grow(&mut self, n_new: usize) {
        if n_new <= self.n {
            return;
        }
        let words = n_new.div_ceil(64);
        self.bits.resize(words, 0);
        self.wbase.resize(words, 0);
        self.wcap.resize(words, 0);
        let mut c = LayerCounts::default();
        for v in self.n..n_new {
            let o = self.arena.len() as u32;
            self.insert_bit(v as u32, [o, 0], &mut c);
        }
        self.n = n_new;
    }

    /// Replace rows: `batch` holds (id, offset, length) into `buf`, each row sorted without
    /// repeats. A row equal to the base's is dropped from the layer.
    pub fn apply(&mut self, base: &Csr<'_>, batch: &[(u32, u32, u32)], buf: &[u32]) -> LayerCounts {
        let mut c = LayerCounts::default();
        for &(v, o, l) in batch {
            debug_assert!((v as usize) < self.n);
            let row = &buf[o as usize..(o + l) as usize];
            c.rows_written += 1;
            let base_eq = (v as usize) < self.nb && base.row(v) == row;
            if self.bit(v) {
                if base_eq {
                    self.clear_bit(v, &mut c);
                    continue;
                }
                let s = self.slot_of(v);
                let [so, sl] = self.slot[s];
                if l <= sl {
                    self.arena[so as usize..(so + l) as usize].copy_from_slice(row);
                    self.dead += (sl - l) as usize;
                    self.slot[s] = [so, l];
                    c.rows_in_place += 1;
                } else {
                    let no = self.arena.len() as u32;
                    self.arena.extend_from_slice(row);
                    self.dead += sl as usize;
                    self.slot[s] = [no, l];
                    c.rows_appended += 1;
                }
            } else if !base_eq {
                let no = self.arena.len() as u32;
                self.arena.extend_from_slice(row);
                self.insert_bit(v, [no, l], &mut c);
                c.rows_appended += 1;
            }
        }
        if (self.dead > 4096 && self.dead > self.arena.len() / 2) || (self.dead_slots > 4096 && self.dead_slots > self.slot.len() / 2) {
            self.compact();
            c.compactions += 1;
        }
        c
    }

    /// Slot blocks laid out again in word order (capacity: the next power of two), rows copied
    /// into a fresh arena in that order.
    fn compact(&mut self) {
        let mut slot = Vec::with_capacity(self.live * 2);
        let mut a = Vec::with_capacity(self.arena.len() - self.dead);
        for w in 0..self.bits.len() {
            let cnt = self.bits[w].count_ones() as usize;
            let ob = self.wbase[w] as usize;
            let cap = if cnt == 0 { 0 } else { cnt.next_power_of_two().min(64) };
            let nb = slot.len();
            for i in 0..cnt {
                let [o, l] = self.slot[ob + i];
                slot.push([a.len() as u32, l]);
                a.extend_from_slice(&self.arena[o as usize..(o + l) as usize]);
            }
            slot.resize(nb + cap, [0, 0]);
            self.wbase[w] = nb as u32;
            self.wcap[w] = cap as u8;
        }
        self.slot = slot;
        self.arena = a;
        self.dead = 0;
        self.dead_slots = 0;
    }

    /// The rows over `base`.
    pub fn rows<'a>(&'a self, base: Csr<'a>) -> LRows<'a> {
        LRows { base, l: self }
    }

    /// Replaced rows, by id (for the checks and the census).
    pub fn replaced_ids(&self) -> Vec<u32> {
        let mut v = Vec::with_capacity(self.live);
        for (w, &b) in self.bits.iter().enumerate() {
            let mut b = b;
            while b != 0 {
                v.push((w as u32) << 6 | b.trailing_zeros());
                b &= b - 1;
            }
        }
        v
    }
}

/// A worktree's rows: the base's unless its bit is set (L5's layered read: one bit test, then a
/// base row or the replacement row at the word's block offset plus the bit's rank in the word).
#[derive(Clone, Copy)]
pub struct LRows<'a> {
    base: Csr<'a>,
    l: &'a MutLayer,
}

impl Rows for LRows<'_> {
    fn n(&self) -> usize {
        self.l.n
    }
    #[inline(always)]
    fn row(&self, v: u32) -> &[u32] {
        let w = (v >> 6) as usize;
        let m = 1u64 << (v & 63);
        // SAFETY: `v < n` (callers' ids are seeds or targets below n); bits/wbase have n/64
        // words; a set bit's slot lies inside its word's block.
        let b = unsafe { *self.l.bits.get_unchecked(w) };
        if b & m == 0 {
            self.base.row(v)
        } else {
            let r = unsafe { *self.l.wbase.get_unchecked(w) } + (b & (m - 1)).count_ones();
            let [o, len] = unsafe { *self.l.slot.get_unchecked(r as usize) };
            unsafe { self.l.arena.get_unchecked(o as usize..(o + len) as usize) }
        }
    }
    fn edges(&self) -> usize {
        self.base.edges() + self.l.live_edges()
    }
}

/// One edit's changes to a lattice fact's inputs: edges and seeds that left and came. The rows
/// and seed bits passed with it are already the new ones.
#[derive(Default, Debug, Clone)]
pub struct Edit {
    pub del: Vec<(u32, u32)>,
    pub ins: Vec<(u32, u32)>,
    pub seeds_del: Vec<u32>,
    pub seeds_ins: Vec<u32>,
}

/// Per-worker scratch, sized to the largest id space and cleared sparsely after each use.
#[derive(Default)]
pub struct Scratch {
    mark: Vec<u64>,
    inv: Vec<u64>,
    aux: Vec<u64>,
    buckets: Vec<Vec<u32>>,
    a: Vec<u32>,
    b: Vec<u32>,
    c: Vec<(u32, u32)>,
}

impl Scratch {
    fn fit(&mut self, n: usize) {
        let w = n.div_ceil(64);
        if self.mark.len() < w {
            self.mark.resize(w, 0);
            self.inv.resize(w, 0);
        }
    }
    fn bucket(&mut self, l: u32) -> &mut Vec<u32> {
        let l = l as usize;
        if self.buckets.len() <= l {
            self.buckets.resize_with(l + 1, Vec::new);
        }
        &mut self.buckets[l]
    }
}

#[inline(always)]
fn tst(b: &[u64], v: u32) -> bool {
    b[(v >> 6) as usize] >> (v & 63) & 1 == 1
}
#[inline(always)]
fn put(b: &mut [u64], v: u32) {
    b[(v >> 6) as usize] |= 1 << (v & 63);
}
#[inline(always)]
fn clr(b: &mut [u64], v: u32) {
    b[(v >> 6) as usize] &= !(1 << (v & 63));
}

/// `min_level` maintenance's counts (L8).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LevelCounts {
    /// Ids re-checked (a deleted witness edge's target, a deleted seed, an invalidated node's
    /// successor one level up).
    pub checked: u64,
    pub invalidated: u64,
    /// Rounds of the invalidation phase (levels holding a check).
    pub inval_rounds: u64,
    /// Transposed edges read by the support checks (up to the first witness).
    pub support_edges: u64,
    /// Forward edges read to find invalidated nodes' successors.
    pub inval_edges: u64,
    /// Transposed edges read to re-derive the invalidated ids.
    pub rederive_edges: u64,
    pub rederived: u64,
    /// Rounds of the propagation phase, their frontier and forward edges.
    pub prop_rounds: u64,
    pub prop_frontier: u64,
    pub prop_edges: u64,
    /// Values that differ after the edit; reachability toggles among them.
    pub changed: u64,
    pub toggled: u64,
}

/// `min_level` after an edit: `fwd`/`rev` and `seed` are the new graph and seed bits, `lvl` the
/// old exact levels (grown with `INF` for new ids); on return `lvl` is exact for the new graph.
pub fn level_update<F: Rows, B: Rows>(fwd: &F, rev: &B, seed: &[u64], lvl: &mut [u32], e: &Edit, sc: &mut Scratch) -> LevelCounts {
    let n = fwd.n();
    debug_assert_eq!(rev.n(), n);
    debug_assert_eq!(lvl.len(), n);
    sc.fit(n);
    let mut c = LevelCounts::default();
    // (1) Invalidation, level by level. `mark`: queued for a check; `inv`: invalidated.
    let mut lo = u32::MAX;
    let mut hi = 0u32;
    let mut queue = |sc: &mut Scratch, v: u32, l: u32, lo: &mut u32, hi: &mut u32| {
        if !tst(&sc.mark, v) {
            put(&mut sc.mark, v);
            sc.a.push(v);
            sc.bucket(l).push(v);
            *lo = (*lo).min(l);
            *hi = (*hi).max(l);
        }
    };
    for &(s, t) in &e.del {
        let ls = lvl[s as usize];
        if ls != INF && lvl[t as usize] == ls + 1 {
            queue(sc, t, ls + 1, &mut lo, &mut hi);
        }
    }
    for &s in &e.seeds_del {
        if lvl[s as usize] == 0 && !tst(seed, s) {
            queue(sc, s, 0, &mut lo, &mut hi);
        }
    }
    let mut inv: Vec<u32> = std::mem::take(&mut sc.b);
    let mut l = lo;
    while lo != u32::MAX && l <= hi {
        let cur = std::mem::take(sc.bucket(l));
        if !cur.is_empty() {
            c.inval_rounds += 1;
        }
        for &v in &cur {
            c.checked += 1;
            debug_assert_eq!(lvl[v as usize], l);
            let supported = if l == 0 {
                tst(seed, v)
            } else {
                let mut ok = false;
                for &p in rev.row(v) {
                    c.support_edges += 1;
                    if lvl[p as usize] == l - 1 && (crate::mutant::on(75) || !tst(&sc.inv, p)) {
                        ok = true;
                        break;
                    }
                }
                ok
            };
            if supported {
                continue;
            }
            put(&mut sc.inv, v);
            inv.push(v);
            let r = fwd.row(v);
            c.inval_edges += r.len() as u64;
            for &w in r {
                if lvl[w as usize] == l + 1 && !tst(&sc.inv, w) {
                    queue(sc, w, l + 1, &mut lo, &mut hi);
                }
            }
        }
        let mut cur = cur;
        cur.clear();
        sc.buckets[l as usize] = cur;
        l += 1;
    }
    for &v in &sc.a {
        clr(&mut sc.mark, v);
    }
    sc.a.clear();
    c.invalidated = inv.len() as u64;
    // (2) Re-derive: the invalidated ids lose their values, then take 1 + the least level among
    // their predecessors (all of them settled: outside the invalidated set, levels did not rise).
    let old: Vec<(u32, u32)> = inv.iter().map(|&v| (v, lvl[v as usize])).collect();
    for &v in &inv {
        lvl[v as usize] = INF;
    }
    sc.c.clear();
    for &v in &inv {
        let mut best = if tst(seed, v) { 0 } else { INF };
        if best != 0 {
            for &p in rev.row(v) {
                c.rederive_edges += 1;
                let lp = lvl[p as usize];
                if lp != INF && lp + 1 < best {
                    best = lp + 1;
                }
            }
        }
        if best != INF {
            sc.c.push((v, best));
        }
    }
    for &v in &inv {
        clr(&mut sc.inv, v);
    }
    c.rederived = sc.c.len() as u64;
    // (3) Decreases, in rounds of uniform level from the lowest candidate. `mark`: invalidated
    // (to tell a newly reached id from one that came back); `inv`: lowered at least once.
    for &(v, _) in &old {
        put(&mut sc.mark, v);
    }
    let (mut lo, mut hi) = (u32::MAX, 0u32);
    let mut came = 0u64;
    let mut lower = |sc: &mut Scratch, lvl: &mut [u32], v: u32, l: u32, lo: &mut u32, hi: &mut u32| {
        let x = lvl[v as usize];
        if l < x {
            if x == INF && !tst(&sc.mark, v) {
                came += 1;
            }
            lvl[v as usize] = l;
            sc.bucket(l).push(v);
            if !tst(&sc.inv, v) {
                put(&mut sc.inv, v);
                sc.a.push(v);
            }
            *lo = (*lo).min(l);
            *hi = (*hi).max(l);
        }
    };
    let cands = std::mem::take(&mut sc.c);
    for &(v, l) in &cands {
        lower(sc, lvl, v, l, &mut lo, &mut hi);
    }
    sc.c = cands;
    for &(s, t) in &e.ins {
        let ls = lvl[s as usize];
        if ls != INF {
            lower(sc, lvl, t, ls + 1, &mut lo, &mut hi);
        }
    }
    for &s in &e.seeds_ins {
        if tst(seed, s) {
            lower(sc, lvl, s, 0, &mut lo, &mut hi);
        }
    }
    let mut l = lo;
    while lo != u32::MAX && l <= hi {
        let cur = std::mem::take(sc.bucket(l));
        let mut width = 0;
        for &v in &cur {
            if lvl[v as usize] != l {
                continue;
            }
            width += 1;
            let r = fwd.row(v);
            c.prop_edges += r.len() as u64;
            for &w in r {
                lower(sc, lvl, w, l + 1, &mut lo, &mut hi);
            }
        }
        if width > 0 {
            c.prop_rounds += 1;
            c.prop_frontier += width;
        }
        let mut cur = cur;
        cur.clear();
        sc.buckets[l as usize] = cur;
        l += 1;
    }
    // Values that changed: the invalidated ids whose value differs from before, and every lowered
    // id that was not invalidated (lowering only decreases a value it did not raise).
    for &(v, o) in &old {
        if lvl[v as usize] != o {
            c.changed += 1;
            if lvl[v as usize] == INF {
                c.toggled += 1;
            }
        }
    }
    for &v in &sc.a {
        if !tst(&sc.mark, v) {
            c.changed += 1;
        }
        clr(&mut sc.inv, v);
    }
    c.toggled += came;
    for &(v, _) in &old {
        clr(&mut sc.mark, v);
    }
    sc.a.clear();
    inv.clear();
    sc.b = inv;
    c
}

/// `set` maintenance's counts.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SetCounts {
    pub over_deleted: u64,
    pub over_rounds: u64,
    pub over_edges: u64,
    pub rederive_edges: u64,
    pub rederived: u64,
    pub prop_rounds: u64,
    pub prop_frontier: u64,
    pub prop_edges: u64,
    pub toggled: u64,
}

/// `set` after an edit (DRed restricted to the reached ids a deletion can reach): `fwd`/`rev`
/// and `seed` are new, `set` the old reached bits; on return `set` is exact for the new graph.
pub fn set_update<F: Rows, B: Rows>(fwd: &F, rev: &B, seed: &[u64], set: &mut [u64], e: &Edit, sc: &mut Scratch) -> SetCounts {
    let n = fwd.n();
    sc.fit(n);
    let mut c = SetCounts::default();
    // (1) Over-delete: from the targets of deleted edges out of reached ids and from deleted
    // seeds, forward through the new rows, over reached ids. Every id that loses reachability is
    // in it: its old paths all crossed a deleted edge or seed, and past the last one they use
    // edges that stayed.
    let mut over: Vec<u32> = std::mem::take(&mut sc.b);
    let mut front: Vec<u32> = std::mem::take(&mut sc.a);
    let mut add = |sc: &mut Scratch, v: u32, front: &mut Vec<u32>, over: &mut Vec<u32>| {
        if tst(set, v) && !tst(&sc.mark, v) {
            put(&mut sc.mark, v);
            front.push(v);
            over.push(v);
        }
    };
    for &(s, t) in &e.del {
        if tst(set, s) {
            add(sc, t, &mut front, &mut over);
        }
    }
    for &s in &e.seeds_del {
        if !tst(seed, s) {
            add(sc, s, &mut front, &mut over);
        }
    }
    let mut next: Vec<u32> = Vec::new();
    while !front.is_empty() && !crate::mutant::on(77) {
        c.over_rounds += 1;
        for &v in &front {
            let r = fwd.row(v);
            c.over_edges += r.len() as u64;
            for &w in r {
                add(sc, w, &mut next, &mut over);
            }
        }
        std::mem::swap(&mut front, &mut next);
        next.clear();
    }
    c.over_deleted = over.len() as u64;
    // (2) Re-derive: an over-deleted id that is a seed or has a reached predecessor outside.
    for &v in &over {
        clr(set, v);
    }
    front.clear();
    for &v in &over {
        let mut ok = tst(seed, v);
        if !ok {
            for &p in rev.row(v) {
                c.rederive_edges += 1;
                if tst(set, p) {
                    ok = true;
                    break;
                }
            }
        }
        if ok {
            front.push(v);
        }
    }
    c.rederived = front.len() as u64;
    // (3) Forward rounds from the re-derived ids, the inserted edges' targets and new seeds.
    for &v in &front {
        put(set, v);
    }
    for &(s, t) in &e.ins {
        if tst(set, s) && !tst(set, t) {
            put(set, t);
            front.push(t);
        }
    }
    for &s in &e.seeds_ins {
        if tst(seed, s) && !tst(set, s) {
            put(set, s);
            front.push(s);
        }
    }
    let mut came: u64 = 0;
    while !front.is_empty() {
        c.prop_rounds += 1;
        c.prop_frontier += front.len() as u64;
        for &v in &front {
            if !tst(&sc.mark, v) {
                came += 1;
            }
            let r = fwd.row(v);
            c.prop_edges += r.len() as u64;
            for &w in r {
                if !tst(set, w) {
                    put(set, w);
                    next.push(w);
                }
            }
        }
        std::mem::swap(&mut front, &mut next);
        next.clear();
    }
    let gone = over.iter().filter(|&&v| !tst(set, v)).count() as u64;
    c.toggled = gone + came;
    for &v in &over {
        clr(&mut sc.mark, v);
    }
    over.clear();
    sc.b = over;
    front.clear();
    sc.a = front;
    c
}

/// `set_update_bf`'s counts.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BfCounts {
    pub candidates: u64,
    pub searches: u64,
    pub found: u64,
    pub search_rounds: u64,
    pub search_visited: u64,
    pub search_edges: u64,
    pub dead: u64,
    pub succ_edges: u64,
    pub prop_rounds: u64,
    pub prop_edges: u64,
    pub toggled: u64,
}

/// `set` after an edit by backward/forward checks (module docs): `fwd`/`rev` and `seed` are new,
/// `set` the old reached bits; on return `set` is exact for the new graph.
pub fn set_update_bf<F: Rows, B: Rows>(fwd: &F, rev: &B, seed: &[u64], set: &mut [u64], e: &Edit, sc: &mut Scratch) -> BfCounts {
    let n = fwd.n();
    sc.fit(n);
    if sc.aux.len() < sc.mark.len() {
        sc.aux.resize(sc.mark.len(), 0);
    }
    let mut c = BfCounts::default();
    // `mark`: visited by the current search; `inv`: dead; `aux`: proven reachable.
    let mut cand: Vec<u32> = Vec::new();
    for &(s, t) in &e.del {
        if tst(set, s) && tst(set, t) {
            cand.push(t);
        }
    }
    for &s in &e.seeds_del {
        if !tst(seed, s) && tst(set, s) {
            cand.push(s);
        }
    }
    c.candidates = cand.len() as u64;
    let mut dead: Vec<u32> = std::mem::take(&mut sc.b);
    let mut proven: Vec<u32> = Vec::new();
    let mut visited: Vec<u32> = std::mem::take(&mut sc.a);
    let (mut front, mut next): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    while let Some(x) = cand.pop() {
        if tst(&sc.inv, x) || tst(&sc.aux, x) || !tst(set, x) {
            continue;
        }
        if tst(seed, x) {
            put(&mut sc.aux, x);
            proven.push(x);
            continue;
        }
        c.searches += 1;
        visited.clear();
        front.clear();
        put(&mut sc.mark, x);
        visited.push(x);
        front.push(x);
        let mut found = false;
        'search: while !front.is_empty() {
            c.search_rounds += 1;
            for &v in &front {
                for &p in rev.row(v) {
                    c.search_edges += 1;
                    if tst(&sc.mark, p) || tst(&sc.inv, p) {
                        continue;
                    }
                    if tst(seed, p) || tst(&sc.aux, p) || (crate::mutant::on(79) && tst(set, p)) {
                        found = true;
                        break 'search;
                    }
                    put(&mut sc.mark, p);
                    visited.push(p);
                    next.push(p);
                }
            }
            std::mem::swap(&mut front, &mut next);
            next.clear();
        }
        next.clear();
        c.search_visited += visited.len() as u64;
        for &v in &visited {
            clr(&mut sc.mark, v);
        }
        if found {
            c.found += 1;
            put(&mut sc.aux, x);
            proven.push(x);
            continue;
        }
        for &v in &visited {
            put(&mut sc.inv, v);
            dead.push(v);
            let r = fwd.row(v);
            c.succ_edges += r.len() as u64;
            for &w in r {
                if tst(set, w) && !tst(&sc.inv, w) && !tst(&sc.aux, w) {
                    cand.push(w);
                }
            }
        }
    }
    for &v in &proven {
        clr(&mut sc.aux, v);
    }
    let mut gone = 0u64;
    for &v in &dead {
        clr(&mut sc.inv, v);
        if tst(set, v) {
            clr(set, v);
            gone += 1;
        }
    }
    c.dead = gone;
    // Forward rounds from the inserted edges and the new seeds.
    front.clear();
    for &(s, t) in &e.ins {
        if tst(set, s) && !tst(set, t) {
            put(set, t);
            front.push(t);
        }
    }
    for &s in &e.seeds_ins {
        if tst(seed, s) && !tst(set, s) {
            put(set, s);
            front.push(s);
        }
    }
    let mut came = front.len() as u64;
    while !front.is_empty() {
        c.prop_rounds += 1;
        for &v in &front {
            let r = fwd.row(v);
            c.prop_edges += r.len() as u64;
            for &w in r {
                if !tst(set, w) {
                    put(set, w);
                    next.push(w);
                }
            }
        }
        came += next.len() as u64;
        std::mem::swap(&mut front, &mut next);
        next.clear();
    }
    // A dead id that a new edge reaches again came back: count it once, as a change of neither.
    let back = dead.iter().filter(|&&v| tst(set, v)).count() as u64;
    c.toggled = gone + came - 2 * back.min(gone);
    dead.clear();
    sc.b = dead;
    visited.clear();
    sc.a = visited;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::round::{csr_columns, naive, transpose, Lattice, Values};

    /// A small deterministic generator (xorshift).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A maintained graph: base CSR + layer, transposed base + layer, seeds, levels, set bits;
    /// `edges` is the reference edge set.
    struct G {
        nb: usize,
        boff: Vec<u32>,
        btgt: Vec<u32>,
        roff: Vec<u32>,
        rtgt: Vec<u32>,
        fwd: MutLayer,
        rev: MutLayer,
        edges: std::collections::BTreeSet<(u32, u32)>,
        seeds: std::collections::BTreeSet<u32>,
        lvl: Vec<u32>,
        set: Vec<u64>,
        set_bf: Vec<u64>,
        sc: Scratch,
    }

    fn rows_of(n: usize, edges: &std::collections::BTreeSet<(u32, u32)>, rev: bool) -> Vec<Vec<u32>> {
        let mut r = vec![Vec::new(); n];
        for &(s, t) in edges {
            if rev {
                r[t as usize].push(s);
            } else {
                r[s as usize].push(t);
            }
        }
        for x in &mut r {
            x.sort_unstable();
        }
        r
    }

    impl G {
        fn new(nb: usize, edges: Vec<(u32, u32)>, seeds: Vec<u32>) -> G {
            let (boff, btgt) = csr_columns(nb, &edges);
            let (roff, rtgt) = transpose(&Csr::new(&boff, &btgt, nb));
            let mut g = G { nb, boff, btgt, roff, rtgt, fwd: MutLayer::new(nb), rev: MutLayer::new(nb), edges: edges.into_iter().collect(), seeds: seeds.into_iter().collect(), lvl: Vec::new(), set: Vec::new(), set_bf: Vec::new(), sc: Scratch::default() };
            let sd: Vec<u32> = g.seeds.iter().copied().collect();
            let Values::Levels(l) = naive(&g.fwd.rows(Csr::new(&g.boff, &g.btgt, nb)), &sd, Lattice::MinLevel) else { unreachable!() };
            let Values::Bits(b) = naive(&g.fwd.rows(Csr::new(&g.boff, &g.btgt, nb)), &sd, Lattice::Set) else { unreachable!() };
            g.lvl = l;
            g.set_bf = b.clone();
            g.set = b;
            g
        }

        fn n(&self) -> usize {
            self.fwd.n()
        }

        fn seed_bits(&self) -> Vec<u64> {
            let mut b = vec![0u64; self.n().div_ceil(64)];
            for &s in &self.seeds {
                put(&mut b, s);
            }
            b
        }

        /// Apply an edit: `grow` new ids, edges removed/added, seeds removed/added; then both
        /// lattices maintained and checked against the naive fixpoint.
        fn edit(&mut self, grow: usize, del: &[(u32, u32)], ins: &[(u32, u32)], sdel: &[u32], sins: &[u32]) -> (LevelCounts, SetCounts) {
            let n = self.n() + grow;
            self.fwd.grow(n);
            self.rev.grow(n);
            self.lvl.resize(n, INF);
            self.set.resize(n.div_ceil(64), 0);
            self.set_bf.resize(n.div_ceil(64), 0);
            let mut e = Edit::default();
            for &x in del {
                if self.edges.remove(&x) {
                    e.del.push(x);
                }
            }
            for &x in ins {
                if self.edges.insert(x) {
                    e.ins.push(x);
                }
            }
            for &s in sdel {
                if self.seeds.remove(&s) {
                    e.seeds_del.push(s);
                }
            }
            for &s in sins {
                if self.seeds.insert(s) {
                    e.seeds_ins.push(s);
                }
            }
            // The changed rows, forward and transposed, as batches.
            for (rev, layer) in [(false, &mut self.fwd), (true, &mut self.rev)] {
                let rows = rows_of(n, &self.edges, rev);
                let mut ids: Vec<u32> = e.del.iter().chain(&e.ins).map(|&(s, t)| if rev { t } else { s }).collect();
                ids.sort_unstable();
                ids.dedup();
                let (mut batch, mut buf) = (Vec::new(), Vec::new());
                for v in ids {
                    batch.push((v, buf.len() as u32, rows[v as usize].len() as u32));
                    buf.extend_from_slice(&rows[v as usize]);
                }
                let base = if rev { Csr::new(&self.roff, &self.rtgt, self.nb) } else { Csr::new(&self.boff, &self.btgt, self.nb) };
                layer.apply(&base, &batch, &buf);
                let lr = layer.rows(base);
                for v in 0..n as u32 {
                    assert_eq!(lr.row(v), rows[v as usize].as_slice(), "layer row {v} (rev {rev})");
                }
            }
            let seed = self.seed_bits();
            let fwd = self.fwd.rows(Csr::new(&self.boff, &self.btgt, self.nb));
            let rev = self.rev.rows(Csr::new(&self.roff, &self.rtgt, self.nb));
            let lc = level_update(&fwd, &rev, &seed, &mut self.lvl, &e, &mut self.sc);
            let sc = set_update(&fwd, &rev, &seed, &mut self.set, &e, &mut self.sc);
            set_update_bf(&fwd, &rev, &seed, &mut self.set_bf, &e, &mut self.sc);
            let sd: Vec<u32> = self.seeds.iter().copied().collect();
            let Values::Levels(l) = naive(&fwd, &sd, Lattice::MinLevel) else { unreachable!() };
            let Values::Bits(b) = naive(&fwd, &sd, Lattice::Set) else { unreachable!() };
            assert_eq!(self.lvl, l, "levels after edit");
            assert_eq!(self.set, b, "set after edit");
            assert_eq!(self.set_bf, b, "set (B/F) after edit");
            assert!([&self.sc.mark, &self.sc.inv, &self.sc.aux].iter().all(|x| x.iter().all(|w| *w == 0)), "scratch cleared");
            (lc, sc)
        }
    }

    #[test]
    fn deleting_the_entry_of_a_cycle_unreaches_it() {
        // 0 → 1 → 2 → 3 → 1 (a cycle reached through 0 → 1 only).
        let mut g = G::new(4, vec![(0, 1), (1, 2), (2, 3), (3, 1)], vec![0]);
        assert_eq!(g.lvl, [0, 1, 2, 3]);
        let (lc, sc) = g.edit(0, &[(0, 1)], &[], &[], &[]);
        assert_eq!(g.lvl, [0, INF, INF, INF]);
        assert_eq!((lc.invalidated, lc.toggled, sc.toggled), (3, 3, 3));
    }

    #[test]
    fn a_second_witness_keeps_the_level_and_reparents() {
        // 0 → 1, 0 → 2, 1 → 3, 2 → 3, 3 → 4: deleting 1 → 3 keeps 3 at level 2 (witness 2).
        let mut g = G::new(5, vec![(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)], vec![0]);
        let (lc, _) = g.edit(0, &[(1, 3)], &[], &[], &[]);
        assert_eq!((lc.checked, lc.invalidated, lc.changed), (1, 0, 0));
        // Then deleting 0 → 2 re-parents 3 and 4 deeper through a longer path 0 → 1 → 5 → 3.
        let (lc, _) = g.edit(1, &[(0, 2)], &[(1, 5), (5, 3)], &[], &[]);
        assert_eq!(g.lvl, [0, 1, INF, 3, 4, 2]);
        assert!(lc.invalidated >= 2);
    }

    #[test]
    fn removing_a_seed_and_adding_another() {
        let mut g = G::new(4, vec![(0, 1), (2, 3), (3, 1)], vec![0, 2]);
        assert_eq!(g.lvl, [0, 1, 0, 1]);
        g.edit(0, &[], &[], &[0], &[]);
        assert_eq!(g.lvl, [INF, 2, 0, 1]);
        g.edit(0, &[], &[], &[2], &[1]);
        assert_eq!(g.lvl, [INF, 0, INF, INF]);
    }

    #[test]
    fn a_cycle_does_not_support_itself() {
        // 0 → 1 → 2 → 1, 2 → 3; deleting 0 → 1 must not keep 1/2 alive through each other even
        // though 2 → 1 exists (1 is at level 1, 2 at level 2: the levels break the cycle).
        let mut g = G::new(4, vec![(0, 1), (1, 2), (2, 1), (2, 3)], vec![0]);
        g.edit(0, &[(0, 1)], &[], &[], &[]);
        assert_eq!(g.lvl, [0, INF, INF, INF]);
    }

    #[test]
    fn random_edits_equal_the_naive_fixpoint() {
        for seed in 1..=40u64 {
            let mut r = Rng(seed * 0x9E37_79B9_7F4A_7C15);
            let n = 2 + r.below(200) as usize;
            let m = r.below(4 * n as u64) as usize;
            let edges: Vec<(u32, u32)> = (0..m).map(|_| (r.below(n as u64) as u32, r.below(n as u64) as u32)).collect();
            let seeds: Vec<u32> = (0..1 + r.below(3)).map(|_| r.below(n as u64) as u32).collect();
            let mut g = G::new(n, edges, seeds);
            for _ in 0..30 {
                let grow = if r.below(4) == 0 { r.below(5) as usize } else { 0 };
                let nn = (g.n() + grow) as u64;
                let cur: Vec<(u32, u32)> = g.edges.iter().copied().collect();
                let del: Vec<(u32, u32)> = (0..r.below(6)).filter_map(|_| (!cur.is_empty()).then(|| cur[r.below(cur.len() as u64) as usize])).collect();
                let ins: Vec<(u32, u32)> = (0..r.below(6)).map(|_| (r.below(nn) as u32, r.below(nn) as u32)).collect();
                let sv: Vec<u32> = g.seeds.iter().copied().collect();
                let sdel: Vec<u32> = if r.below(5) == 0 && !sv.is_empty() { vec![sv[r.below(sv.len() as u64) as usize]] } else { vec![] };
                let sins: Vec<u32> = if r.below(5) == 0 { vec![r.below(nn) as u32] } else { vec![] };
                g.edit(grow, &del, &ins, &sdel, &sins);
            }
            // The layer holds only rows that differ from the base.
            let base = Csr::new(&g.boff, &g.btgt, g.nb);
            let rows = rows_of(g.n(), &g.edges, false);
            for v in g.fwd.replaced_ids() {
                assert!(v as usize >= g.nb || base.row(v) != rows[v as usize].as_slice(), "row {v} equal to the base kept");
            }
        }
    }

    #[test]
    fn a_long_chain_cut_in_the_middle_and_restored() {
        let n = 3000;
        let edges: Vec<(u32, u32)> = (0..n - 1).map(|i| (i as u32, i as u32 + 1)).collect();
        let mut g = G::new(n, edges, vec![0]);
        let (lc, sc) = g.edit(0, &[(1500, 1501)], &[], &[], &[]);
        assert_eq!((lc.invalidated, sc.over_deleted, lc.toggled), (1499, 1499, 1499));
        let (lc, _) = g.edit(0, &[], &[(0, 2000)], &[], &[]);
        assert_eq!(lc.changed, 1000);
        g.edit(0, &[], &[(1500, 1501)], &[], &[]);
    }
}
