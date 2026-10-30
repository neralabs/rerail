//! The semiring round (LATTICE.md L5): a monotone fixpoint over a closed lattice, evaluated as
//! rounds of sparse matrix × frontier on columns read in place.
//!
//! - Edges: CSR, `u32` offsets and `u32` targets, each row sorted and without repeats (`Csr`), or
//!   a base CSR with per-worktree replacement rows (`Layered`, L10): a row is the base's unless the
//!   worktree's bit for it is set, in which case it is the delta CSR's row at the bit's rank.
//! - Values: a dense array by `u32` id. `min_level` (min, +1 over ℕ ∪ {∞}): `u32`, `INF` = ∞.
//!   `set` (∨, ∧ over booleans): a bitset.
//! - Frontier: a bitset; the sequential kernel also keeps it as a sorted `u32` list while it is
//!   sparse (fewer than `n / SPARSE_DIV` ids), so sparse rounds do not scan the bitset.
//! - Parallel within a round: equal ranges of frontier words claimed through one counter; a
//!   derivation is joined into the shared value array by the lattice's join as an atomic
//!   (`fetch_min` / `fetch_or`), and the worker whose join changed the value sets the id's bit
//!   in the next frontier. One barrier per round. Workers allocate nothing: their private state is
//!   two counters. The result (values, and every count of `Round`) is a function of the inputs
//!   alone (L4): joins commute and are idempotent, and the count of changed values is counted
//!   once per id, by the one worker whose join changed it.
//!
//! The inner loop is: read a frontier word, for each set bit read one row (two offsets, a slice
//! of targets), for each target one load and, rarely, one atomic. No hash probe, no allocation,
//! no pointer chasing, no dynamic dispatch (`Rows` is monomorphised).
//!
//! Nothing here depends on the rest of the crate: the module reads `u32`/`u64` slices only.
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

pub const INF: u32 = u32::MAX;
/// The sequential kernel keeps a sorted list while the frontier is below `n / SPARSE_DIV`.
pub const SPARSE_DIV: usize = 64;

/// Out-edges by source id.
pub trait Rows: Sync {
    fn n(&self) -> usize;
    fn row(&self, v: u32) -> &[u32];
    /// Every target id < `n` (checked once, so the kernels index without bounds checks).
    fn edges(&self) -> usize;
}

/// A compressed sparse row: row `v` is `tgt[off[v]..off[v + 1]]`.
#[derive(Clone, Copy)]
pub struct Csr<'a> {
    off: &'a [u32],
    tgt: &'a [u32],
}

impl<'a> Csr<'a> {
    /// Checks the layout once: offsets ascending from 0 to `tgt.len()`, every row strictly
    /// ascending (sorted, no repeats), every target below `n` (the target id space).
    pub fn new(off: &'a [u32], tgt: &'a [u32], n: usize) -> Csr<'a> {
        assert!(!off.is_empty() && off[0] == 0 && *off.last().unwrap() as usize == tgt.len(), "csr offsets");
        for w in off.windows(2) {
            assert!(w[0] <= w[1], "csr offsets ascending");
            let r = &tgt[w[0] as usize..w[1] as usize];
            assert!(r.windows(2).all(|p| p[0] < p[1]), "csr row sorted, no repeats");
            assert!(r.last().is_none_or(|&t| (t as usize) < n), "csr target in range");
        }
        Csr { off, tgt }
    }

    /// The same columns `new` already checked (a maintained base, read again per edit): no
    /// second pass over the layout.
    pub fn trusted(off: &'a [u32], tgt: &'a [u32]) -> Csr<'a> {
        debug_assert!(!off.is_empty() && *off.last().unwrap() as usize == tgt.len());
        Csr { off, tgt }
    }

    pub fn rows(&self) -> usize {
        self.off.len() - 1
    }
}

impl Rows for Csr<'_> {
    fn n(&self) -> usize {
        self.off.len() - 1
    }
    #[inline(always)]
    fn row(&self, v: u32) -> &[u32] {
        let v = v as usize;
        debug_assert!(v + 1 < self.off.len());
        // SAFETY: `new` checked the offsets are ascending and within `tgt`; `v < n` is the
        // caller's (every id the kernels read is a seed or a target, both checked below `n`).
        unsafe {
            let a = *self.off.get_unchecked(v) as usize;
            let b = *self.off.get_unchecked(v + 1) as usize;
            self.tgt.get_unchecked(a..b)
        }
    }
    fn edges(&self) -> usize {
        self.tgt.len()
    }
}

/// A base CSR over the whole id space, and one worktree's replacement rows: `bits` has a bit per
/// id (set: the row is replaced), `rank[w]` counts the set bits before word `w`, and `delta` holds
/// the replacement rows in id order. State per worktree: n/64 words + n/64 ranks + its rows.
#[derive(Clone, Copy)]
pub struct Layered<'a> {
    base: Csr<'a>,
    bits: &'a [u64],
    rank: &'a [u32],
    delta: Csr<'a>,
}

impl<'a> Layered<'a> {
    pub fn new(base: Csr<'a>, bits: &'a [u64], rank: &'a [u32], delta: Csr<'a>) -> Layered<'a> {
        let n = base.rows();
        assert_eq!(bits.len(), n.div_ceil(64), "layer bits");
        assert_eq!(rank.len(), bits.len(), "layer ranks");
        let mut r = 0u32;
        for (w, b) in bits.iter().enumerate() {
            assert_eq!(rank[w], r, "layer rank");
            r += b.count_ones();
        }
        assert_eq!(r as usize, delta.rows(), "one replacement row per set bit");
        Layered { base, bits, rank, delta }
    }
}

impl Rows for Layered<'_> {
    fn n(&self) -> usize {
        self.base.rows()
    }
    #[inline(always)]
    fn row(&self, v: u32) -> &[u32] {
        let w = (v >> 6) as usize;
        let m = 1u64 << (v & 63);
        // SAFETY: `v < n` (as for `Csr`), and `bits`/`rank` have n/64 words (checked in `new`).
        let b = unsafe { *self.bits.get_unchecked(w) };
        if b & m == 0 {
            self.base.row(v)
        } else {
            let r = unsafe { *self.rank.get_unchecked(w) } + (b & (m - 1)).count_ones();
            self.delta.row(r)
        }
    }
    fn edges(&self) -> usize {
        self.base.edges() + self.delta.edges()
    }
}

/// The catalogue entries this operator evaluates (L2), integer carriers only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Lattice {
    /// ∨ over booleans; a round is the boolean semiring (∨, ∧).
    Set,
    /// min over ℕ ∪ {∞}; a round is the tropical semiring (min, +) with unit edge weights. With
    /// every seed at 0 the value is the BFS depth: Y2's level.
    MinLevel,
}

/// A fixpoint's result: `Levels` for `MinLevel` (`INF` = not reached), `Bits` for `Set`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Values {
    Levels(Vec<u32>),
    Bits(Vec<u64>),
}

impl Values {
    pub fn reached(&self, v: u32) -> bool {
        match self {
            Values::Levels(l) => l[v as usize] != INF,
            Values::Bits(b) => b[(v >> 6) as usize] >> (v & 63) & 1 == 1,
        }
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: plain integers, viewed as bytes.
        unsafe {
            match self {
                Values::Levels(l) => std::slice::from_raw_parts(l.as_ptr() as *const u8, l.len() * 4),
                Values::Bits(b) => std::slice::from_raw_parts(b.as_ptr() as *const u8, b.len() * 8),
            }
        }
    }
}

/// One round's algebraic counts: frontier width, edges scanned (the derivations), values changed
/// (the next frontier's width), and whether the sequential kernel iterated a sorted list.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Round {
    pub width: u64,
    pub edges: u64,
    pub changed: u64,
    /// A pull round (`parallel_dir`): `edges` counts transposed edges read.
    pub pull: bool,
}

/// A fixpoint's counts (L8) and, for the parallel kernel, its synchronisation: per worker the
/// time spent waiting at the round barrier, and the bytes it allocated (always 0: workers own
/// two counters).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Counts {
    pub seeds: u64,
    pub rounds: Vec<Round>,
    /// Sequential kernel: rounds that iterated the sorted list instead of the bitset.
    pub list_rounds: u64,
    pub barrier_wait_ms: Vec<f64>,
    pub worker_alloc_bytes: Vec<u64>,
    /// Wall time before round 0 (allocation, seeds; for the parallel kernel also the spawn, up to
    /// worker 0's first round), then per round (worker 0, barrier included). Not algebraic.
    pub start_ms: f64,
    pub round_ms: Vec<f64>,
}

impl Counts {
    pub fn edges(&self) -> u64 {
        self.rounds.iter().map(|r| r.edges).sum()
    }
    /// The algebraic part only (equal across kernels and thread counts).
    pub fn algebraic(&self) -> (u64, &[Round]) {
        (self.seeds, &self.rounds)
    }
}

fn check_seeds(n: usize, seeds: &[u32]) {
    assert!(seeds.iter().all(|&s| (s as usize) < n), "seed in range");
}

/// The fixpoint from `seeds` (every seed at the lattice's bottom-plus-one: level 0 / reached), on
/// one thread, plain memory.
pub fn sequential<R: Rows>(g: &R, seeds: &[u32], lat: Lattice) -> (Values, Counts) {
    match lat {
        Lattice::MinLevel => seq_kernel(g, seeds, SeqLevels(vec![INF; g.n()])),
        Lattice::Set => seq_kernel(g, seeds, SeqBits(vec![0; g.n().div_ceil(64)])),
    }
}

trait SeqVal {
    /// Join `lvl` into `t`; true if the value changed.
    fn join(&mut self, t: u32, lvl: u32) -> bool;
    fn done(self) -> Values;
}

struct SeqLevels(Vec<u32>);
impl SeqVal for SeqLevels {
    #[inline(always)]
    fn join(&mut self, t: u32, lvl: u32) -> bool {
        // SAFETY: `t < n` (seeds checked, targets checked by `Csr::new`).
        let x = unsafe { self.0.get_unchecked_mut(t as usize) };
        if lvl < *x {
            *x = lvl;
            true
        } else {
            false
        }
    }
    fn done(self) -> Values {
        Values::Levels(self.0)
    }
}

struct SeqBits(Vec<u64>);
impl SeqVal for SeqBits {
    #[inline(always)]
    fn join(&mut self, t: u32, _: u32) -> bool {
        let m = 1u64 << (t & 63);
        // SAFETY: as above, n/64 words.
        let x = unsafe { self.0.get_unchecked_mut((t >> 6) as usize) };
        let new = *x & m == 0;
        *x |= m;
        new
    }
    fn done(self) -> Values {
        Values::Bits(self.0)
    }
}

fn seq_kernel<R: Rows, V: SeqVal>(g: &R, seeds: &[u32], mut val: V) -> (Values, Counts) {
    let t0 = Instant::now();
    let n = g.n();
    check_seeds(n, seeds);
    let cap = (n / SPARSE_DIV).max(1);
    let words = n.div_ceil(64);
    let mut cur = vec![0u64; words];
    let mut next = vec![0u64; words];
    // The frontier as a sorted list while it fits `cap` (`None`: iterate the bitset).
    let mut list: Option<Vec<u32>> = Some(Vec::with_capacity(cap));
    let mut nlist: Vec<u32> = Vec::with_capacity(cap);
    let mut c = Counts::default();
    let mut width = 0u64;
    for &s in seeds {
        if val.join(s, 0) {
            cur[(s >> 6) as usize] |= 1 << (s & 63);
            width += 1;
            if let Some(l) = list.as_mut() {
                if l.len() < cap {
                    l.push(s);
                } else {
                    list = None;
                }
            }
        }
    }
    c.seeds = width;
    c.start_ms = t0.elapsed().as_secs_f64() * 1e3;
    let mut k = 0u32;
    while width > 0 {
        let tr = Instant::now();
        let lvl = k + 1;
        let (mut edges, mut changed) = (0u64, 0u64);
        nlist.clear();
        let mut overflow = false;
        let mut relax = |v: u32, next: &mut [u64], nlist: &mut Vec<u32>| {
            let r = g.row(v);
            edges += r.len() as u64;
            for &t in r {
                if val.join(t, lvl) {
                    // SAFETY: `t < n`.
                    unsafe { *next.get_unchecked_mut((t >> 6) as usize) |= 1 << (t & 63) };
                    changed += 1;
                    if nlist.len() < cap {
                        nlist.push(t);
                    } else {
                        overflow = true;
                    }
                }
            }
        };
        match list.as_mut() {
            Some(l) => {
                c.list_rounds += 1;
                l.sort_unstable();
                for &v in l.iter() {
                    cur[(v >> 6) as usize] = 0;
                    relax(v, &mut next, &mut nlist);
                }
            }
            None => {
                for w in 0..words {
                    let mut b = cur[w];
                    if b == 0 {
                        continue;
                    }
                    cur[w] = 0;
                    while b != 0 {
                        let v = (w as u32) << 6 | b.trailing_zeros();
                        b &= b - 1;
                        relax(v, &mut next, &mut nlist);
                    }
                }
            }
        }
        c.rounds.push(Round { width, edges, changed, pull: false });
        c.round_ms.push(tr.elapsed().as_secs_f64() * 1e3);
        std::mem::swap(&mut cur, &mut next);
        match (&mut list, overflow) {
            (Some(l), false) => std::mem::swap(l, &mut nlist),
            (None, false) => list = Some(std::mem::take(&mut nlist)),
            (_, true) => {
                if let Some(l) = list.take() {
                    nlist = l;
                }
            }
        }
        if nlist.capacity() < cap {
            nlist = Vec::with_capacity(cap);
        }
        width = changed;
        k += 1;
    }
    (val.done(), c)
}

trait ParVal: Sync {
    /// Join `lvl` into `t` (any worker); true if the value changed.
    fn join(&self, t: u32, lvl: u32) -> bool;
    /// The reached ids of word `wi` (within `valid`), as a mask.
    fn visited(&self, wi: usize, valid: u64) -> u64;
    /// Set the ids of `nb` in word `wi` to `lvl`: only the word's owner calls it, in a pull round.
    fn set_owned(&self, wi: usize, nb: u64, lvl: u32);
    /// Whether joining `lvl` into `t` would change it (a read, no write).
    fn improves(&self, t: u32, lvl: u32) -> bool;
    /// Join `lvl` into `t` by its owner (no other worker writes `t`'s word this phase); true if
    /// the value changed.
    fn join_owned(&self, t: u32, lvl: u32) -> bool;
}

struct ParLevels(Vec<AtomicU32>);
impl ParVal for ParLevels {
    #[inline(always)]
    fn join(&self, t: u32, lvl: u32) -> bool {
        // SAFETY: `t < n`.
        let x = unsafe { self.0.get_unchecked(t as usize) };
        x.load(Relaxed) > lvl && x.fetch_min(lvl, Relaxed) > lvl
    }
    fn visited(&self, wi: usize, valid: u64) -> u64 {
        let mut m = 0u64;
        let mut c = valid;
        while c != 0 {
            let b = c.trailing_zeros();
            c &= c - 1;
            if self.0[wi * 64 + b as usize].load(Relaxed) != INF {
                m |= 1 << b;
            }
        }
        m
    }
    fn set_owned(&self, wi: usize, nb: u64, lvl: u32) {
        let mut c = nb;
        while c != 0 {
            let b = c.trailing_zeros();
            c &= c - 1;
            self.0[wi * 64 + b as usize].store(lvl, Relaxed);
        }
    }
    #[inline(always)]
    fn improves(&self, t: u32, lvl: u32) -> bool {
        unsafe { self.0.get_unchecked(t as usize) }.load(Relaxed) > lvl
    }
    #[inline(always)]
    fn join_owned(&self, t: u32, lvl: u32) -> bool {
        let x = unsafe { self.0.get_unchecked(t as usize) };
        if x.load(Relaxed) > lvl {
            x.store(lvl, Relaxed);
            true
        } else {
            false
        }
    }
}

struct ParBits(Vec<AtomicU64>);
impl ParVal for ParBits {
    #[inline(always)]
    fn join(&self, t: u32, _: u32) -> bool {
        let m = 1u64 << (t & 63);
        // SAFETY: `t < n`, n/64 words.
        let x = unsafe { self.0.get_unchecked((t >> 6) as usize) };
        x.load(Relaxed) & m == 0 && x.fetch_or(m, Relaxed) & m == 0
    }
    fn visited(&self, wi: usize, valid: u64) -> u64 {
        self.0[wi].load(Relaxed) & valid
    }
    fn set_owned(&self, wi: usize, nb: u64, _: u32) {
        let x = &self.0[wi];
        x.store(x.load(Relaxed) | nb, Relaxed);
    }
    #[inline(always)]
    fn improves(&self, t: u32, _: u32) -> bool {
        unsafe { self.0.get_unchecked((t >> 6) as usize) }.load(Relaxed) >> (t & 63) & 1 == 0
    }
    #[inline(always)]
    fn join_owned(&self, t: u32, _: u32) -> bool {
        let m = 1u64 << (t & 63);
        let x = unsafe { self.0.get_unchecked((t >> 6) as usize) };
        let w = x.load(Relaxed);
        if w & m == 0 {
            x.store(w | m, Relaxed);
            true
        } else {
            false
        }
    }
}

/// A sense-reversing barrier that spins briefly, then yields (the machine is shared: a pure spin
/// would starve the other workers when threads outnumber free cores).
struct SpinBarrier {
    n: usize,
    count: AtomicUsize,
    sense: AtomicBool,
}

impl SpinBarrier {
    fn new(n: usize) -> SpinBarrier {
        SpinBarrier { n, count: AtomicUsize::new(0), sense: AtomicBool::new(false) }
    }
    fn wait(&self, local: &mut bool) {
        *local = !*local;
        if self.count.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1 == self.n {
            self.count.store(0, Relaxed);
            self.sense.store(*local, std::sync::atomic::Ordering::Release);
        } else {
            let mut i = 0u32;
            while self.sense.load(std::sync::atomic::Ordering::Acquire) != *local {
                if i < 2000 {
                    std::hint::spin_loop();
                    i += 1;
                } else {
                    std::thread::yield_now();
                }
            }
        }
    }
}

/// Words of frontier per work unit: about 16 units per worker, at least one word.
fn chunk_words(words: usize, threads: usize) -> usize {
    words.div_ceil(threads * 16).max(1)
}

/// Direction switch (Beamer et al., direction-optimizing BFS): pull when the frontier's out-edges
/// exceed the unexplored ids' out-edges / `ALPHA`; back to push when the frontier is below
/// n / `BETA`. Decided from counts only, so the same at every thread count.
const ALPHA: u64 = 14;
const BETA: u64 = 24;

/// The fixpoint on `threads` workers (scoped threads, spawned once per fixpoint), every round a
/// push: frontier rows scanned, targets joined.
pub fn parallel<R: Rows>(g: &R, seeds: &[u32], lat: Lattice, threads: usize) -> (Values, Counts) {
    par_entry(g, None, seeds, lat, threads)
}

/// As `parallel`, with pull rounds when the frontier is wide: each worker owns equal ranges of
/// target ids and reads their rows of the transposed CSR `rev` until one source is in the
/// frontier. A pull round writes only owned values: no atomic read-modify-write, no shared line
/// written by two workers.
pub fn parallel_dir<R: Rows>(g: &R, rev: &Csr<'_>, seeds: &[u32], lat: Lattice, threads: usize) -> (Values, Counts) {
    assert_eq!(rev.rows(), g.n(), "transpose over the same ids");
    par_entry(g, Some(rev), seeds, lat, threads)
}

fn par_entry<R: Rows>(g: &R, rev: Option<&Csr<'_>>, seeds: &[u32], lat: Lattice, threads: usize) -> (Values, Counts) {
    let t0 = Instant::now();
    let n = g.n();
    match lat {
        Lattice::MinLevel => {
            let (v, c) = par_kernel(g, rev, seeds, ParLevels(atomic_u32(vec![INF; n])), threads, t0);
            (Values::Levels(plain_u32(v.0)), c)
        }
        Lattice::Set => {
            let (v, c) = par_kernel(g, rev, seeds, ParBits(atomic_u64(vec![0; n.div_ceil(64)])), threads, t0);
            (Values::Bits(plain_u64(v.0)), c)
        }
    }
}

// In place, no copy: `AtomicU32`/`AtomicU64` have the in-memory representation of `u32`/`u64`.
fn atomic_u32(v: Vec<u32>) -> Vec<AtomicU32> {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU32, v.len(), v.capacity()) }
}
fn plain_u32(v: Vec<AtomicU32>) -> Vec<u32> {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut u32, v.len(), v.capacity()) }
}
fn atomic_u64(v: Vec<u64>) -> Vec<AtomicU64> {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU64, v.len(), v.capacity()) }
}
fn plain_u64(v: Vec<AtomicU64>) -> Vec<u64> {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut u64, v.len(), v.capacity()) }
}

#[repr(align(128))]
#[derive(Default)]
struct Slot {
    edges: AtomicU64,
    changed: AtomicU64,
    /// Out-edges of the ids this worker added to the next frontier (direction switch only).
    fdeg: AtomicU64,
}

fn par_kernel<R: Rows, V: ParVal>(g: &R, rev: Option<&Csr<'_>>, seeds: &[u32], val: V, threads: usize, t0: Instant) -> (V, Counts) {
    assert!(threads >= 1);
    let n = g.n();
    check_seeds(n, seeds);
    let words = n.div_ceil(64);
    let total_edges = g.edges() as u64;
    // Round k: frontier `bits[k % 3]`, next `bits[(k + 1) % 3]`, and `bits[(k + 2) % 3]` (round
    // k - 1's frontier, read by nobody now) cleared for round k + 1.
    let bits: [Vec<AtomicU64>; 3] = [atomic_u64(vec![0; words]), atomic_u64(vec![0; words]), atomic_u64(vec![0; words])];
    let (mut width, mut fdeg0) = (0u64, 0u64);
    for &s in seeds {
        if val.join(s, 0) {
            bits[0][(s >> 6) as usize].fetch_or(1 << (s & 63), Relaxed);
            width += 1;
            fdeg0 += g.row(s).len() as u64;
        }
    }
    let cw = chunk_words(words, threads);
    let units = words.div_ceil(cw);
    let last_valid = if n % 64 == 0 { u64::MAX } else { (1u64 << (n % 64)) - 1 };
    // Per parity: the work-unit counter and the per-worker counts of the round.
    let ctr = [AtomicUsize::new(0), AtomicUsize::new(0)];
    let slots: [Vec<Slot>; 2] = [(0..threads).map(|_| Slot::default()).collect(), (0..threads).map(|_| Slot::default()).collect()];
    let barrier = SpinBarrier::new(threads);
    let rounds = parking_lot::Mutex::new(Vec::new());
    let waits = parking_lot::Mutex::new(vec![0f64; threads]);
    let times = parking_lot::Mutex::new((0f64, Vec::new()));
    let seeds_width = width;
    std::thread::scope(|s| {
        for w in 0..threads {
            let (g, val, bits, ctr, slots, barrier, rounds, waits, times) = (g, &val, &bits, &ctr, &slots, &barrier, &rounds, &waits, &times);
            s.spawn(move || {
                let mut sense = false;
                let mut width = seeds_width;
                let (mut fdeg, mut explored, mut pull) = (fdeg0, fdeg0, false);
                let mut wait = 0f64;
                let mut tr = Instant::now();
                let start_ms = (tr - t0).as_secs_f64() * 1e3;
                let mut round_ms = Vec::new();
                let mut k = 0usize;
                while width > 0 {
                    if rev.is_some() {
                        if !pull && fdeg > (total_edges - explored.min(total_edges)) / ALPHA {
                            pull = true;
                        } else if pull && width < n as u64 / BETA {
                            pull = false;
                        }
                    }
                    let p = k & 1;
                    let (cur, next, old) = (&bits[k % 3], &bits[(k + 1) % 3], &bits[(k + 2) % 3]);
                    let lvl = k as u32 + 1;
                    let (mut edges, mut changed, mut nfdeg) = (0u64, 0u64, 0u64);
                    loop {
                        let u = ctr[p].fetch_add(1, Relaxed);
                        if u >= units {
                            break;
                        }
                        let (a, b) = (u * cw, ((u + 1) * cw).min(words));
                        for wi in a..b {
                            // SAFETY: `wi < words`.
                            unsafe { old.get_unchecked(wi) }.store(0, Relaxed);
                            if pull {
                                let rev = rev.unwrap();
                                let valid = if wi + 1 == words { last_valid } else { u64::MAX };
                                let mut cand = !val.visited(wi, valid) & valid;
                                let mut nb = 0u64;
                                while cand != 0 {
                                    let bit = cand.trailing_zeros();
                                    cand &= cand - 1;
                                    let v = (wi as u32) << 6 | bit;
                                    for &src in rev.row(v) {
                                        edges += 1;
                                        if unsafe { cur.get_unchecked((src >> 6) as usize) }.load(Relaxed) >> (src & 63) & 1 == 1 {
                                            nb |= 1 << bit;
                                            changed += 1;
                                            nfdeg += g.row(v).len() as u64;
                                            break;
                                        }
                                    }
                                }
                                if nb != 0 {
                                    val.set_owned(wi, nb, lvl);
                                    unsafe { next.get_unchecked(wi) }.store(nb, Relaxed);
                                }
                                continue;
                            }
                            let mut x = unsafe { cur.get_unchecked(wi) }.load(Relaxed);
                            while x != 0 {
                                let v = (wi as u32) << 6 | x.trailing_zeros();
                                x &= x - 1;
                                let r = g.row(v);
                                edges += r.len() as u64;
                                for &t in r {
                                    if val.join(t, lvl) {
                                        unsafe { next.get_unchecked((t >> 6) as usize) }.fetch_or(1 << (t & 63), Relaxed);
                                        changed += 1;
                                        if rev.is_some() {
                                            nfdeg += g.row(t).len() as u64;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    slots[p][w].edges.store(edges, Relaxed);
                    slots[p][w].changed.store(changed, Relaxed);
                    slots[p][w].fdeg.store(nfdeg, Relaxed);
                    // The other parity's counter is free: every worker left its claim loop
                    // before the previous barrier.
                    if w == 0 {
                        ctr[p ^ 1].store(0, Relaxed);
                    }
                    let tw = Instant::now();
                    barrier.wait(&mut sense);
                    wait += tw.elapsed().as_secs_f64() * 1e3;
                    // Every worker sums the round's counts (the same numbers): the next width.
                    let mut tot = Round { width, edges: 0, changed: 0, pull };
                    fdeg = 0;
                    for sl in &slots[p] {
                        tot.edges += sl.edges.load(Relaxed);
                        tot.changed += sl.changed.load(Relaxed);
                        fdeg += sl.fdeg.load(Relaxed);
                    }
                    explored += fdeg;
                    width = tot.changed;
                    if w == 0 {
                        rounds.lock().push(tot);
                        round_ms.push(tr.elapsed().as_secs_f64() * 1e3);
                        tr = Instant::now();
                    }
                    k += 1;
                }
                waits.lock()[w] = wait;
                if w == 0 {
                    *times.lock() = (start_ms, round_ms);
                }
            });
        }
    });
    let (start_ms, round_ms) = times.into_inner();
    let c = Counts { seeds: seeds_width, rounds: rounds.into_inner(), list_rounds: 0, barrier_wait_ms: waits.into_inner(), worker_alloc_bytes: vec![0; threads], start_ms, round_ms };
    (val, c)
}

/// As `parallel`, with work units that are equal ranges of the round's *edges* rather than of its
/// frontier ids: a cluster of high-degree ids (the module nodes, adjacent in id order) is split
/// across workers, a single long row too. Two barriers per round: (A) the frontier bitset becomes
/// a sorted id list with the prefix sums of its rows' lengths, each word range written at the
/// offset its counts give; (B) each edge range finds its first id by binary search and joins
/// its part of the rows. The per-range counts that place phase A are kept per worker during
/// phase B (no shared counter is written by two workers).
pub fn parallel_edges<R: Rows>(g: &R, seeds: &[u32], lat: Lattice, threads: usize) -> (Values, Counts) {
    let t0 = Instant::now();
    let n = g.n();
    match lat {
        Lattice::MinLevel => {
            let (v, c) = edge_kernel(g, seeds, ParLevels(atomic_u32(vec![INF; n])), threads, t0);
            (Values::Levels(plain_u32(v.0)), c)
        }
        Lattice::Set => {
            let (v, c) = edge_kernel(g, seeds, ParBits(atomic_u64(vec![0; n.div_ceil(64)])), threads, t0);
            (Values::Bits(plain_u64(v.0)), c)
        }
    }
}

fn edge_kernel<R: Rows, V: ParVal>(g: &R, seeds: &[u32], val: V, threads: usize, t0: Instant) -> (V, Counts) {
    assert!(threads >= 1);
    let n = g.n();
    check_seeds(n, seeds);
    let words = n.div_ceil(64);
    let cw = chunk_words(words, threads);
    let units = words.div_ceil(cw);
    let span = (cw * 64) as u32;
    let eunits = (threads * 16) as u64;
    // Frontier bitsets as in `par_kernel` (three, rotating); the frontier as a list, with the
    // prefix sums of its rows' lengths.
    let bits: [Vec<AtomicU64>; 3] = [atomic_u64(vec![0; words]), atomic_u64(vec![0; words]), atomic_u64(vec![0; words])];
    let list: Vec<AtomicU32> = atomic_u32(vec![0; n]);
    let pre: Vec<AtomicU64> = atomic_u64(vec![0; n + 1]);
    // Per parity, per worker, per word range: (ids, row lengths) added to the next frontier.
    let cnt: [Vec<AtomicU64>; 2] = [atomic_u64(vec![0; 2 * threads * units]), atomic_u64(vec![0; 2 * threads * units])];
    let mut width = 0u64;
    for &s in seeds {
        if val.join(s, 0) {
            bits[0][(s >> 6) as usize].fetch_or(1 << (s & 63), Relaxed);
            let u = (s / span) as usize;
            cnt[0][2 * u].fetch_add(1, Relaxed);
            cnt[0][2 * u + 1].fetch_add(g.row(s).len() as u64, Relaxed);
            width += 1;
        }
    }
    let ctr = [AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)];
    let slots: [Vec<Slot>; 2] = [(0..threads).map(|_| Slot::default()).collect(), (0..threads).map(|_| Slot::default()).collect()];
    let barrier = SpinBarrier::new(threads);
    let rounds = parking_lot::Mutex::new(Vec::new());
    let waits = parking_lot::Mutex::new(vec![0f64; threads]);
    let times = parking_lot::Mutex::new((0f64, Vec::new()));
    let seeds_width = width;
    std::thread::scope(|s| {
        for w in 0..threads {
            let (g, val, bits, list, pre, cnt, ctr, slots, barrier, rounds, waits, times) = (g, &val, &bits, &list, &pre, &cnt, &ctr, &slots, &barrier, &rounds, &waits, &times);
            s.spawn(move || {
                let mut sense = false;
                let mut width = seeds_width;
                let mut wait = 0f64;
                let mut tr = Instant::now();
                let start_ms = (tr - t0).as_secs_f64() * 1e3;
                let mut round_ms = Vec::new();
                let mut k = 0usize;
                // This worker's copy of the word ranges' offsets (ids, edges): its only buffer.
                let mut offs: Vec<(u64, u64)> = vec![(0, 0); units + 1];
                let mut wait_at = |sense: &mut bool| {
                    let tw = Instant::now();
                    barrier.wait(sense);
                    wait += tw.elapsed().as_secs_f64() * 1e3;
                };
                while width > 0 {
                    let p = k & 1;
                    let (cur, next, old) = (&bits[k % 3], &bits[(k + 1) % 3], &bits[(k + 2) % 3]);
                    let lvl = k as u32 + 1;
                    // Offsets of each word range in the list and in the edges, from the counts
                    // every worker left for this frontier (the same sums in every worker).
                    let c = &cnt[p];
                    for u in 0..units {
                        let (mut ids, mut es) = offs[u];
                        for x in 0..threads {
                            ids += c[2 * (x * units + u)].load(Relaxed);
                            es += c[2 * (x * units + u) + 1].load(Relaxed);
                        }
                        offs[u + 1] = (ids, es);
                    }
                    // Phase A: the list and its prefix sums, by word range.
                    loop {
                        let u = ctr[0].fetch_add(1, Relaxed);
                        if u >= units {
                            break;
                        }
                        let (mut pos, mut acc) = offs[u];
                        for wi in u * cw..((u + 1) * cw).min(words) {
                            unsafe { old.get_unchecked(wi) }.store(0, Relaxed);
                            let mut x = unsafe { cur.get_unchecked(wi) }.load(Relaxed);
                            while x != 0 {
                                let v = (wi as u32) << 6 | x.trailing_zeros();
                                x &= x - 1;
                                unsafe { list.get_unchecked(pos as usize) }.store(v, Relaxed);
                                unsafe { pre.get_unchecked(pos as usize) }.store(acc, Relaxed);
                                pos += 1;
                                acc += g.row(v).len() as u64;
                            }
                        }
                    }
                    wait_at(&mut sense);
                    let total = offs[units].1;
                    // This worker's counts for the next frontier start at zero.
                    let nc = &cnt[p ^ 1];
                    for u in 0..units {
                        nc[2 * (w * units + u)].store(0, Relaxed);
                        nc[2 * (w * units + u) + 1].store(0, Relaxed);
                    }
                    // Phase B: equal ranges of the round's edges.
                    let (mut edges, mut changed) = (0u64, 0u64);
                    loop {
                        let j = ctr[1].fetch_add(1, Relaxed) as u64;
                        if j >= eunits {
                            break;
                        }
                        let (a, b) = (j * total / eunits, (j + 1) * total / eunits);
                        if a == b {
                            continue;
                        }
                        // The last id whose row starts at or before `a`.
                        let (mut lo, mut hi) = (0usize, width as usize);
                        while hi - lo > 1 {
                            let m = (lo + hi) / 2;
                            if unsafe { pre.get_unchecked(m) }.load(Relaxed) <= a {
                                lo = m;
                            } else {
                                hi = m;
                            }
                        }
                        let mut i = lo;
                        while i < width as usize {
                            let start = unsafe { pre.get_unchecked(i) }.load(Relaxed);
                            if start >= b {
                                break;
                            }
                            let v = unsafe { list.get_unchecked(i) }.load(Relaxed);
                            let r = g.row(v);
                            let from = a.saturating_sub(start) as usize;
                            let to = ((b - start) as usize).min(r.len());
                            if from < to {
                                edges += (to - from) as u64;
                                for &t in &r[from..to] {
                                    if val.join(t, lvl) {
                                        unsafe { next.get_unchecked((t >> 6) as usize) }.fetch_or(1 << (t & 63), Relaxed);
                                        changed += 1;
                                        let u = (t / span) as usize;
                                        let cell = 2 * (w * units + u);
                                        nc[cell].store(nc[cell].load(Relaxed) + 1, Relaxed);
                                        nc[cell + 1].store(nc[cell + 1].load(Relaxed) + g.row(t).len() as u64, Relaxed);
                                    }
                                }
                            }
                            i += 1;
                        }
                    }
                    slots[p][w].edges.store(edges, Relaxed);
                    slots[p][w].changed.store(changed, Relaxed);
                    if w == 0 {
                        ctr[0].store(0, Relaxed);
                    }
                    wait_at(&mut sense);
                    if w == 0 {
                        ctr[1].store(0, Relaxed);
                    }
                    let mut tot = Round { width, edges: 0, changed: 0, pull: false };
                    for sl in &slots[p] {
                        tot.edges += sl.edges.load(Relaxed);
                        tot.changed += sl.changed.load(Relaxed);
                    }
                    width = tot.changed;
                    if w == 0 {
                        rounds.lock().push(tot);
                        round_ms.push(tr.elapsed().as_secs_f64() * 1e3);
                        tr = Instant::now();
                    }
                    k += 1;
                }
                drop(wait_at);
                waits.lock()[w] = wait;
                if w == 0 {
                    *times.lock() = (start_ms, round_ms);
                }
            });
        }
    });
    let (start_ms, round_ms) = times.into_inner();
    let c = Counts { seeds: seeds_width, rounds: rounds.into_inner(), list_rounds: 0, barrier_wait_ms: waits.into_inner(), worker_alloc_bytes: vec![((units + 1) * 16) as u64; threads], start_ms, round_ms };
    (val, c)
}

/// As `parallel`, with every value written by one owner only (L6's "workers own their output
/// slices"): the id space is cut into `16 × threads` word-aligned partitions. Phase 1: workers
/// scan frontier word ranges and append each target the join would improve (a read of the value,
/// no write) to their own buffer for the target's partition. Phase 2: each partition is claimed by
/// one worker, which joins every buffered target into its values and next-frontier words with
/// plain stores. No atomic read-modify-write on values, no line written by two workers; the price
/// is the buffers (private bytes per worker ≈ its share of a round's candidate targets × 4 B) and
/// a second barrier per round.
pub fn parallel_owner<R: Rows>(g: &R, seeds: &[u32], lat: Lattice, threads: usize) -> (Values, Counts) {
    let t0 = Instant::now();
    let n = g.n();
    match lat {
        Lattice::MinLevel => {
            let (v, c) = owner_kernel(g, seeds, ParLevels(atomic_u32(vec![INF; n])), threads, t0);
            (Values::Levels(plain_u32(v.0)), c)
        }
        Lattice::Set => {
            let (v, c) = owner_kernel(g, seeds, ParBits(atomic_u64(vec![0; n.div_ceil(64)])), threads, t0);
            (Values::Bits(plain_u64(v.0)), c)
        }
    }
}

/// Per (worker, partition) candidate buffers. Phase 1: worker `w` alone pushes to row `w`. Phase
/// 2: the claimer of partition `p` alone reads and empties column `p`. The phases are separated by
/// barriers, so no buffer is touched by two threads at once.
struct Bufs(Vec<std::cell::UnsafeCell<Vec<u32>>>);
// SAFETY: access is partitioned by phase as above.
unsafe impl Sync for Bufs {}

fn owner_kernel<R: Rows, V: ParVal>(g: &R, seeds: &[u32], val: V, threads: usize, t0: Instant) -> (V, Counts) {
    assert!(threads >= 1);
    let n = g.n();
    check_seeds(n, seeds);
    let words = n.div_ceil(64);
    let cw = chunk_words(words, threads);
    let units = words.div_ceil(cw);
    // Partitions: the same word ranges as the scan units, so a partition's words are its own.
    let span = (cw * 64) as u32;
    let parts = units;
    let bits: [Vec<AtomicU64>; 2] = [atomic_u64(vec![0; words]), atomic_u64(vec![0; words])];
    let mut width = 0u64;
    for &s in seeds {
        if val.join(s, 0) {
            bits[0][(s >> 6) as usize].fetch_or(1 << (s & 63), Relaxed);
            width += 1;
        }
    }
    let bufs = Bufs((0..threads * parts).map(|_| std::cell::UnsafeCell::new(Vec::new())).collect());
    let ctr = [AtomicUsize::new(0), AtomicUsize::new(0)];
    let slots: [Vec<Slot>; 2] = [(0..threads).map(|_| Slot::default()).collect(), (0..threads).map(|_| Slot::default()).collect()];
    let barrier = SpinBarrier::new(threads);
    let rounds = parking_lot::Mutex::new(Vec::new());
    let waits = parking_lot::Mutex::new(vec![0f64; threads]);
    let times = parking_lot::Mutex::new((0f64, Vec::new()));
    let seeds_width = width;
    std::thread::scope(|s| {
        for w in 0..threads {
            let (g, val, bits, bufs, ctr, slots, barrier, rounds, waits, times) = (g, &val, &bits, &bufs, &ctr, &slots, &barrier, &rounds, &waits, &times);
            s.spawn(move || {
                let mut sense = false;
                let mut width = seeds_width;
                let mut wait = 0f64;
                let mut tr = Instant::now();
                let start_ms = (tr - t0).as_secs_f64() * 1e3;
                let mut round_ms = Vec::new();
                let mut k = 0usize;
                let mut wait_at = |sense: &mut bool| {
                    let tw = Instant::now();
                    barrier.wait(sense);
                    wait += tw.elapsed().as_secs_f64() * 1e3;
                };
                while width > 0 {
                    let p = k & 1;
                    let (cur, next) = (&bits[p], &bits[p ^ 1]);
                    let lvl = k as u32 + 1;
                    // Phase 1: scan, buffer the candidates by partition.
                    let mut edges = 0u64;
                    loop {
                        let u = ctr[0].fetch_add(1, Relaxed);
                        if u >= units {
                            break;
                        }
                        for wi in u * cw..((u + 1) * cw).min(words) {
                            let cell = unsafe { cur.get_unchecked(wi) };
                            let mut x = cell.load(Relaxed);
                            if x == 0 {
                                continue;
                            }
                            cell.store(0, Relaxed);
                            while x != 0 {
                                let v = (wi as u32) << 6 | x.trailing_zeros();
                                x &= x - 1;
                                let r = g.row(v);
                                edges += r.len() as u64;
                                for &t in r {
                                    if val.improves(t, lvl) {
                                        // SAFETY: row `w` of the buffers is this worker's in phase 1.
                                        unsafe { &mut *bufs.0.get_unchecked(w * parts + (t / span) as usize).get() }.push(t);
                                    }
                                }
                            }
                        }
                    }
                    if w == 0 {
                        ctr[1].store(0, Relaxed);
                    }
                    wait_at(&mut sense);
                    // Phase 2: each partition joined by its claimer.
                    let mut changed = 0u64;
                    loop {
                        let q = ctr[1].fetch_add(1, Relaxed);
                        if q >= parts {
                            break;
                        }
                        for x in 0..threads {
                            // SAFETY: column `q` is this worker's in phase 2.
                            let b = unsafe { &mut *bufs.0.get_unchecked(x * parts + q).get() };
                            for &t in b.iter() {
                                if val.join_owned(t, lvl) {
                                    let c = unsafe { next.get_unchecked((t >> 6) as usize) };
                                    c.store(c.load(Relaxed) | 1 << (t & 63), Relaxed);
                                    changed += 1;
                                }
                            }
                            b.clear();
                        }
                    }
                    slots[p][w].edges.store(edges, Relaxed);
                    slots[p][w].changed.store(changed, Relaxed);
                    if w == 0 {
                        ctr[0].store(0, Relaxed);
                    }
                    wait_at(&mut sense);
                    let mut tot = Round { width, edges: 0, changed: 0, pull: false };
                    for sl in &slots[p] {
                        tot.edges += sl.edges.load(Relaxed);
                        tot.changed += sl.changed.load(Relaxed);
                    }
                    width = tot.changed;
                    if w == 0 {
                        rounds.lock().push(tot);
                        round_ms.push(tr.elapsed().as_secs_f64() * 1e3);
                        tr = Instant::now();
                    }
                    k += 1;
                }
                drop(wait_at);
                waits.lock()[w] = wait;
                if w == 0 {
                    *times.lock() = (start_ms, round_ms);
                }
            });
        }
    });
    let (start_ms, round_ms) = times.into_inner();
    // Each worker's buffers (row `w`), at their high-water capacity.
    let alloc: Vec<u64> = (0..threads).map(|w| (0..parts).map(|q| unsafe { &*bufs.0[w * parts + q].get() }.capacity() as u64 * 4).sum()).collect();
    let c = Counts { seeds: seeds_width, rounds: rounds.into_inner(), list_rounds: 0, barrier_wait_ms: waits.into_inner(), worker_alloc_bytes: alloc, start_ms, round_ms };
    (val, c)
}

/// The reference fixpoint (naive, Bellman–Ford order: every reached id relaxes every out-edge,
/// until a whole pass changes nothing). For the differential tests.
pub fn naive<R: Rows>(g: &R, seeds: &[u32], lat: Lattice) -> Values {
    let n = g.n();
    let mut lv = vec![INF; n];
    for &s in seeds {
        lv[s as usize] = 0;
    }
    loop {
        let mut changed = false;
        for v in 0..n as u32 {
            if lv[v as usize] == INF {
                continue;
            }
            let nl = match lat {
                Lattice::MinLevel => lv[v as usize] + 1,
                Lattice::Set => 0,
            };
            for &t in g.row(v) {
                if nl < lv[t as usize] {
                    lv[t as usize] = nl;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    match lat {
        Lattice::MinLevel => Values::Levels(lv),
        Lattice::Set => {
            let mut b = vec![0u64; n.div_ceil(64)];
            for (v, l) in lv.iter().enumerate() {
                if *l != INF {
                    b[v >> 6] |= 1 << (v & 63);
                }
            }
            Values::Bits(b)
        }
    }
}

/// A CSR from an edge list (rows sorted, repeats dropped), as owned columns.
pub fn csr_columns(n: usize, edges: &[(u32, u32)]) -> (Vec<u32>, Vec<u32>) {
    let mut e = edges.to_vec();
    e.sort_unstable();
    e.dedup();
    let mut off = vec![0u32; n + 1];
    for &(a, _) in &e {
        off[a as usize + 1] += 1;
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    (off, e.into_iter().map(|(_, b)| b).collect())
}

/// A worktree's replacement rows over a base of `n` rows, as owned columns (bits, ranks, delta
/// offsets, delta targets): `rows` holds (id, sorted targets), ascending by id.
pub fn layer_columns(n: usize, rows: &[(u32, Vec<u32>)]) -> (Vec<u64>, Vec<u32>, Vec<u32>, Vec<u32>) {
    let words = n.div_ceil(64);
    let mut bits = vec![0u64; words];
    let mut off = vec![0u32];
    let mut tgt = Vec::new();
    let mut last = None;
    for (v, r) in rows {
        assert!(last.is_none_or(|l| l < *v), "replacement rows ascending by id");
        last = Some(*v);
        bits[(*v >> 6) as usize] |= 1 << (v & 63);
        tgt.extend_from_slice(r);
        off.push(tgt.len() as u32);
    }
    let mut rank = vec![0u32; words];
    let mut r = 0u32;
    for w in 0..words {
        rank[w] = r;
        r += bits[w].count_ones();
    }
    (bits, rank, off, tgt)
}

/// The transposed CSR of `g` (row `t` lists the sources of edges into `t`, ascending), as owned
/// columns: what a pull round reads.
pub fn transpose<R: Rows>(g: &R) -> (Vec<u32>, Vec<u32>) {
    let n = g.n();
    let mut off = vec![0u32; n + 1];
    for v in 0..n as u32 {
        for &t in g.row(v) {
            off[t as usize + 1] += 1;
        }
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    let mut fill = off.clone();
    let mut tgt = vec![0u32; off[n] as usize];
    // Sources visited in ascending order, so every transposed row comes out sorted.
    for v in 0..n as u32 {
        for &t in g.row(v) {
            tgt[fill[t as usize] as usize] = v;
            fill[t as usize] += 1;
        }
    }
    (off, tgt)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn random_edges(r: &mut Rng, n: usize, m: usize) -> Vec<(u32, u32)> {
        (0..m).map(|_| (r.below(n as u64) as u32, r.below(n as u64) as u32)).collect()
    }

    /// Every kernel at 1–8 threads equals the naive fixpoint; the push kernels' counts are the same
    /// across kernels and thread counts, the direction-optimizing kernel's across thread counts.
    /// Returns the pull rounds the direction-optimizing kernel took.
    fn check<R: Rows>(g: &R, seeds: &[u32]) -> usize {
        let (roff, rtgt) = transpose(g);
        let rev = Csr::new(&roff, &rtgt, g.n());
        let mut pulls = 0;
        for lat in [Lattice::MinLevel, Lattice::Set] {
            let want = naive(g, seeds, lat);
            let (s, sc) = sequential(g, seeds, lat);
            assert_eq!(s, want, "sequential {lat:?}");
            let (d1, dc1) = parallel_dir(g, &rev, seeds, lat, 1);
            assert_eq!(d1, want, "direction-optimizing {lat:?} at 1 thread");
            pulls += dc1.rounds.iter().filter(|r| r.pull).count();
            for t in [1, 2, 3, 4, 8] {
                let (p, pc) = parallel(g, seeds, lat, t);
                assert_eq!(p, want, "parallel {lat:?} at {t} threads");
                assert_eq!(pc.algebraic(), sc.algebraic(), "counts {lat:?} at {t} threads");
                let (e, ec) = parallel_edges(g, seeds, lat, t);
                assert_eq!(e, want, "edge-balanced {lat:?} at {t} threads");
                assert_eq!(ec.algebraic(), sc.algebraic(), "edge-balanced counts {lat:?} at {t} threads");
                let (o, oc) = parallel_owner(g, seeds, lat, t);
                assert_eq!(o, want, "owner-computes {lat:?} at {t} threads");
                assert_eq!(oc.algebraic(), sc.algebraic(), "owner-computes counts {lat:?} at {t} threads");
                let (d, dc) = parallel_dir(g, &rev, seeds, lat, t);
                assert_eq!(d, want, "direction-optimizing {lat:?} at {t} threads");
                assert_eq!(dc.algebraic(), dc1.algebraic(), "direction-optimizing counts {lat:?} at {t} threads");
            }
        }
        pulls
    }

    #[test]
    fn dense_graphs_take_pull_rounds() {
        let mut r = Rng(0xB0770);
        let mut pulls = 0;
        for n in [300, 4000, 9000] {
            let (off, tgt) = csr_columns(n, &random_edges(&mut r, n, 12 * n));
            let g = Csr::new(&off, &tgt, n);
            pulls += check(&g, &[0, 1]);
        }
        assert!(pulls > 0, "no pull round taken");
    }

    #[test]
    fn cycles_self_loops_and_unreached() {
        // 0 → 1 → 2 → 0 (cycle), 2 → 3, 3 → 3 (self loop), 4 → 5 (unreached), 5 → 4.
        let (off, tgt) = csr_columns(7, &[(0, 1), (1, 2), (2, 0), (2, 3), (3, 3), (4, 5), (5, 4), (1, 2)]);
        let g = Csr::new(&off, &tgt, 7);
        check(&g, &[0]);
        check(&g, &[2, 2, 0]);
        check(&g, &[]);
        let (v, c) = sequential(&g, &[0], Lattice::MinLevel);
        assert_eq!(v, Values::Levels(vec![0, 1, 2, 3, INF, INF, INF]));
        assert_eq!(c.rounds.iter().map(|r| r.width).collect::<Vec<_>>(), [1, 1, 1, 1]);
    }

    #[test]
    fn random_graphs_against_naive() {
        let mut r = Rng(0x5EED_1A77);
        for case in 0..60 {
            // Sizes across the 64-bit word boundary and past the sparse-list cap.
            let n = [1, 2, 63, 64, 65, 200, 1000, 5000][case % 8];
            let m = (r.below(4 * n as u64 + 1)) as usize;
            let (off, tgt) = csr_columns(n, &random_edges(&mut r, n, m));
            let g = Csr::new(&off, &tgt, n);
            let seeds: Vec<u32> = (0..r.below(4) + 1).map(|_| r.below(n as u64) as u32).collect();
            check(&g, &seeds);
        }
    }

    #[test]
    fn long_chain_many_rounds() {
        let n = 3000;
        let e: Vec<(u32, u32)> = (0..n as u32 - 1).map(|i| (i, i + 1)).chain([(n as u32 - 1, 0)]).collect();
        let (off, tgt) = csr_columns(n, &e);
        let g = Csr::new(&off, &tgt, n);
        check(&g, &[0]);
        assert_eq!(sequential(&g, &[0], Lattice::MinLevel).1.rounds.len(), n);
    }

    /// A layered graph (base + replacement rows) behaves as the flat graph it denotes.
    #[test]
    fn layered_equals_flat() {
        let mut r = Rng(0xD1FF);
        for _ in 0..30 {
            let n = 1 + r.below(700) as usize;
            let base_e = random_edges(&mut r, n, 3 * n);
            let (boff, btgt) = csr_columns(n, &base_e);
            let base = Csr::new(&boff, &btgt, n);
            // Replace a random subset of rows (some with empty rows).
            let mut rows = Vec::new();
            let mut flat = Vec::new();
            for v in 0..n as u32 {
                if r.below(5) == 0 {
                    let mut row: Vec<u32> = (0..r.below(6)).map(|_| r.below(n as u64) as u32).collect();
                    row.sort_unstable();
                    row.dedup();
                    flat.extend(row.iter().map(|&t| (v, t)));
                    rows.push((v, row));
                } else {
                    flat.extend(base.row(v).iter().map(|&t| (v, t)));
                }
            }
            let (bits, rank, doff, dtgt) = layer_columns(n, &rows);
            let lay = Layered::new(base, &bits, &rank, Csr::new(&doff, &dtgt, n));
            let (foff, ftgt) = csr_columns(n, &flat);
            let fg = Csr::new(&foff, &ftgt, n);
            for v in 0..n as u32 {
                assert_eq!(lay.row(v), fg.row(v));
            }
            let seeds = [r.below(n as u64) as u32];
            check(&lay, &seeds);
            for lat in [Lattice::MinLevel, Lattice::Set] {
                assert_eq!(sequential(&lay, &seeds, lat).0, sequential(&fg, &seeds, lat).0);
            }
        }
    }

    #[test]
    #[should_panic(expected = "csr row sorted")]
    fn unsorted_rows_are_refused() {
        let off = [0u32, 2, 2];
        let tgt = [1u32, 0];
        Csr::new(&off, &tgt, 2);
    }
}
