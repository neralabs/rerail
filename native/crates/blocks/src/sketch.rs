//! One linear sketch of a column's values (decision 0678 step 10): the one place a column's distinct values, the sum
//! of its values' rows squared and its join counters are kept, for every owner of a relation's statistics.
//!
//! Its parts, each a count of rows, so each can be added to and taken from:
//!
//! - **The join counters** (`Counts`): `BUCKETS` counters, a row adding ±1 at its value's bucket (the AMS "tug of war",
//!   read by the planner's join sizes); and, while the column holds at most `FEW` values, each value's rows.
//! - **Exact counts past the few values**, while they stay bounded: a page of `DENSE_WIDTH` counts for values in one
//!   aligned range, else a label set of at most `SPARSE_LIMIT` values. While a column keeps exact counts, its distinct
//!   values and sum of squares are exact.
//! - **Rank counts** past them: each HyperLogLog register's rows at each rank (`REGISTERS` × `RANKS`). A register is
//!   its highest rank with rows, so the registers are those of the live rows, about 6.5% off at 256 registers; the
//!   sum of squares is then read from the join counters.
//!
//! A row is taken out as it was put in (`remove`), so a sketch is always that of its live rows, whatever came and
//! went. **A delta** (`Delta`) is the signed change of a version's rows: each value it touched with its rows added
//! less those taken. A state's statistics are its version's sketch read with its delta (`Sketch::with`), in the
//! delta's size; the delta is folded into the version when they settle (`Sketch::apply`).

use crate::maps::{Room, default_room};
use crate::vector::Vector;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicI32, AtomicI64};

/// The join counters a column keeps.
pub const BUCKETS: usize = 256;
/// The values a column keeps its few exact counts of (`Counts`), at most.
pub const FEW: usize = 64;
const REGISTERS: usize = 256;
/// The ranks a register's rows are counted at; a rank past it is counted at it (a value's chance of one is 2^-32).
const RANKS: usize = 32;
const DENSE_WIDTH: usize = 4096;
const LABEL_LIMIT: i64 = 1_i64 << 32;
const SPARSE_LIMIT: usize = 1024;
/// Slots of a label set at most: twice its values, a power of two. It starts at `SPARSE_FIRST` and doubles while its
/// values pass half its slots.
const SPARSE_SLOTS: usize = 2 * SPARSE_LIMIT;
const SPARSE_FIRST: usize = 128;
/// An empty slot's value (a label set's values are in `0..LABEL_LIMIT`).
const NO_VALUE: i64 = -1;

/// `Sketch::encode`'s format.
const SKETCH_FORMAT: u8 = 1;

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(next: &mut impl FnMut() -> Option<u8>) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = next()?;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// The one hash of a value: the Bloom filters' (`bloom::hash`), so a filter and a sketch see the same keys alike.
#[inline(always)]
pub fn hash(v: i64) -> u64 {
    crate::bloom::hash(v)
}

/// A key's counter and its sign there, from its hash.
#[inline(always)]
pub fn bucket(h: u64) -> (usize, i32) {
    (((h >> 8) & 0xff) as usize, if h & 1 == 0 { 1 } else { -1 })
}

/// A key's register and rank, from its hash.
#[inline(always)]
fn place(h: u64) -> (usize, u8) {
    ((h >> 56) as usize, ((h << 8) | 0x80).leading_zeros() as u8 + 1)
}

/// Its rank counts' cell for a register and rank.
#[inline(always)]
fn cell(r: usize, rho: u8) -> usize {
    r * RANKS + usize::from(rho).min(RANKS) - 1
}

/// `n` zeros in `room` (decision 0478 step 6c: a sketch's arrays are block arrays, not the heap's). A sketch takes the
/// run's room unless it is given one (`Sketch::new_in`): the engine installs the facts layer's, and a crate's own
/// tests below it would otherwise take the heap's.
fn zeros<T: crate::Pod + Copy + Default>(room: &Arc<dyn Room>, n: usize) -> Vector<T> {
    let mut v = Vector::with_capacity_in(Arc::clone(room), n);
    v.resize(n, T::default());
    v
}

/// A column's join-size counts, in a block (decision 0478 step 6c): its counters (`BUCKETS`) and, while it has at
/// most `FEW` values, each value's rows (`[value, rows]`). Its sketch is its one writer, as the rows come; the
/// planner's join sizes and the statistics' keys read it where it lies, held by both (decision 0353: read in place,
/// never copied). Every cell is read and written as an atomic of its width, relaxed: the writer's stores are plain
/// stores, and a reader on another thread reads a value whole.
pub struct Counts {
    counters: Vector<i32>,
    few: Vector<[i64; 2]>,
    /// `few`'s values in use, or `PAST` once the column has had more than `FEW`.
    len: std::sync::atomic::AtomicU32,
}

/// `Counts::len` once its values outgrew `FEW`: the counters alone tell its joins.
const PAST: u32 = u32::MAX;

impl Counts {
    /// No rows: its arrays at their full size, in the run's room, so they never move while they are read.
    pub fn new() -> Counts {
        Counts::new_in(&default_room())
    }

    /// No rows, its arrays in `room`.
    pub fn new_in(room: &Arc<dyn Room>) -> Counts {
        Counts { counters: zeros(room, BUCKETS), few: zeros(room, FEW), len: std::sync::atomic::AtomicU32::new(0) }
    }

    fn counter(&self, b: usize) -> &AtomicI32 {
        assert!(b < BUCKETS);
        // SAFETY: counter `b` of the array, which lives and stays where it is while `self` does; an `i32`'s layout
        // is an `AtomicI32`'s, and every access to it is atomic.
        unsafe { &*self.counters.as_ptr().cast::<AtomicI32>().add(b) }
    }

    fn few_cell(&self, i: usize) -> &[AtomicI64; 2] {
        assert!(i < FEW);
        // SAFETY: as `counter`, for entry `i` of `few`.
        unsafe { &*self.few.as_ptr().cast::<[AtomicI64; 2]>().add(i) }
    }

    /// Adds `n` rows (signed) of the key whose hash is `h` to its counter. Its one writer only.
    #[inline(always)]
    pub fn count_n(&self, h: u64, n: i64) {
        let (b, s) = bucket(h);
        let c = self.counter(b);
        c.store(c.load(Relaxed).wrapping_add(s.wrapping_mul(n as i32)), Relaxed);
    }

    /// Adds a row of the key whose hash is `h` to its counter. Its one writer only.
    #[inline(always)]
    pub fn count(&self, h: u64) {
        self.count_n(h, 1);
    }

    /// Takes a row of the key whose hash is `h` out of its counter: `count`'s inverse (the counters are linear, so a
    /// removed row leaves them as if it was never added). Its one writer only.
    #[inline(always)]
    pub fn uncount(&self, h: u64) {
        self.count_n(h, -1);
    }

    /// Takes `n` rows of value `v` out of the exact counts, while they are kept: its rows left, or `None` where they
    /// are not kept or hold no row of `v`. A value left with no rows keeps its entry, at 0: a reader may be reading
    /// the entries as the column's keys, and keys may hold more than the column, never fewer. Its one writer only.
    pub fn remove_few_n(&self, v: i64, n: i64) -> Option<i64> {
        let len = self.len.load(Relaxed);
        if len == PAST {
            return None;
        }
        let e = (0..len as usize).map(|i| self.few_cell(i)).find(|e| e[0].load(Relaxed) == v)?;
        let rows = e[1].load(Relaxed);
        if rows <= 0 {
            return None;
        }
        let left = (rows - n).max(0);
        e[1].store(left, Relaxed);
        Some(left)
    }

    /// `remove_few_n` of one row.
    pub fn remove_few(&self, v: i64) -> Option<i64> {
        self.remove_few_n(v, 1)
    }

    /// Adds `n` rows of value `v` to the exact counts, while they are kept: false when `v` would be value `FEW + 1`
    /// (nothing changed: the writer then ends them, `end_few`, once it has read them). Its one writer only.
    pub fn add_few_n(&self, v: i64, n: i64) -> bool {
        let len = self.len.load(Relaxed);
        if len == PAST {
            return false;
        }
        for i in 0..len as usize {
            let e = self.few_cell(i);
            if e[0].load(Relaxed) == v {
                e[1].store(e[1].load(Relaxed) + n, Relaxed);
                return true;
            }
        }
        if len as usize == FEW {
            return false;
        }
        let e = self.few_cell(len as usize);
        e[0].store(v, Relaxed);
        e[1].store(n, Relaxed);
        // Released: a reader that sees the length sees the entry.
        self.len.store(len + 1, std::sync::atomic::Ordering::Release);
        true
    }

    /// `add_few_n` of one row.
    pub fn add_few(&self, v: i64) -> bool {
        self.add_few_n(v, 1)
    }

    /// Ends the exact counts: past `FEW` values, the counters alone tell its joins. Its one writer only.
    pub fn end_few(&self) {
        self.len.store(PAST, std::sync::atomic::Ordering::Release);
    }

    /// Each value's rows (`[value, rows]`), while it keeps them, read where they lie.
    pub fn few(&self) -> Option<impl Iterator<Item = [i64; 2]> + '_> {
        let n = self.len.load(std::sync::atomic::Ordering::Acquire);
        (n != PAST).then(|| (0..n as usize).map(|i| self.few_cell(i)).map(|e| [e[0].load(Relaxed), e[1].load(Relaxed)]))
    }

    /// Its values with rows while it keeps them: how many (`few_len` less those `remove_few` left at none).
    pub fn few_live(&self) -> Option<usize> {
        Some(self.few()?.filter(|e| e[1] > 0).count())
    }

    /// Its values while it keeps them: how many.
    pub fn few_len(&self) -> Option<usize> {
        let n = self.len.load(std::sync::atomic::Ordering::Acquire);
        (n != PAST).then_some(n as usize)
    }

    /// `other`'s counters added to these (two columns' counters add to those of both). Its one writer only.
    pub fn add_counters(&self, other: &Counts) {
        for (b, n) in other.counters().enumerate() {
            let c = self.counter(b);
            c.store(c.load(Relaxed).wrapping_add(n), Relaxed);
        }
    }

    /// Counter `b`, read where it lies.
    pub fn counter_at(&self, b: usize) -> i32 {
        self.counter(b).load(Relaxed)
    }

    /// Its counters, read where they lie.
    pub fn counters(&self) -> impl Iterator<Item = i32> + '_ {
        (0..BUCKETS).map(|b| self.counter(b).load(Relaxed))
    }

    /// Its arrays' bytes.
    pub fn bytes(&self) -> usize {
        4 * BUCKETS + 16 * FEW
    }

    /// Whether its arrays are in a block (decision 0478 step 6c).
    pub fn in_blocks(&self) -> bool {
        use crate::maps::Placed;
        self.counters.place().is_some() && self.few.place().is_some()
    }
}

/// A new writer's own counts, from these (a sketch's clone at its first add): written once, by that writer, in a
/// block of its own.
impl Clone for Counts {
    fn clone(&self) -> Counts {
        let made = Counts::new_in(&self.counters.room().cloned().unwrap_or_else(default_room));
        for (b, c) in self.counters().enumerate() {
            made.counter(b).store(c, Relaxed);
        }
        match self.few() {
            Some(few) => {
                let mut n = 0;
                for (i, [v, r]) in few.enumerate() {
                    let e = made.few_cell(i);
                    e[0].store(v, Relaxed);
                    e[1].store(r, Relaxed);
                    n = i + 1;
                }
                made.len.store(n as u32, Relaxed);
            }
            None => made.end_few(),
        }
        made
    }
}

impl Default for Counts {
    fn default() -> Counts {
        Counts::new()
    }
}

/// A page of exact counts: values `base..base + DENSE_WIDTH`.
#[derive(Clone)]
struct Dense {
    base: i64,
    counts: Vector<u32>,
    ndv: usize,
    sq: u64,
}

impl Dense {
    fn at(&self, v: i64) -> Option<usize> {
        v.checked_sub(self.base).and_then(|n| usize::try_from(n).ok()).filter(|&i| i < DENSE_WIDTH)
    }
    fn add_n(&mut self, v: i64, k: u64) -> bool {
        let Some(i) = self.at(v) else { return false };
        let n = &mut self.counts[i];
        if *n == 0 {
            self.ndv += 1;
        }
        self.sq += 2 * u64::from(*n) * k + k * k;
        *n += k as u32;
        true
    }
    fn remove_n(&mut self, v: i64, k: u64) {
        let Some(i) = self.at(v) else { return };
        let n = &mut self.counts[i];
        let k = k.min(u64::from(*n));
        if k == 0 {
            return;
        }
        self.sq -= 2 * u64::from(*n) * k - k * k;
        *n -= k as u32;
        if *n == 0 {
            self.ndv -= 1;
        }
    }
    fn count(&self, v: i64) -> Option<u64> {
        self.at(v).map(|i| u64::from(self.counts[i]))
    }
    fn counts(&self) -> impl Iterator<Item = (i64, u64)> + '_ {
        self.counts.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, &n)| (self.base + i as i64, u64::from(n)))
    }
}

/// A label set's exact counts, open addressing in a block: each slot `[value, rows]`, `[NO_VALUE, 0]` for none.
#[derive(Clone)]
struct Sparse {
    slots: Vector<[i64; 2]>,
    len: usize,
    sq: u64,
    /// Values whose rows were all removed: they keep their slots, at 0.
    zero: usize,
}

impl Sparse {
    fn slot(&self, v: i64) -> usize {
        let m = self.slots.len() - 1;
        let mut at = (hash(v) as usize) & m;
        while self.slots[at][0] != NO_VALUE && self.slots[at][0] != v {
            at = (at + 1) & m;
        }
        at
    }
    /// Room for one more value: its slots doubled in their room, each entry placed again, while its values pass half
    /// of them.
    fn grow(&mut self) {
        if 2 * (self.len + 1) <= self.slots.len() || self.slots.len() >= SPARSE_SLOTS {
            return;
        }
        let room = self.slots.room().cloned().unwrap_or_else(default_room);
        let mut grown = Self { slots: Vector::new_in(room), len: 0, sq: self.sq, zero: self.zero };
        grown.slots.resize(self.slots.len() * 2, [NO_VALUE, 0]);
        for e in self.slots.iter().filter(|e| e[0] != NO_VALUE) {
            let at = grown.slot(e[0]);
            grown.slots[at] = *e;
            grown.len += 1;
        }
        *self = grown;
    }
    fn from_counts(room: &Arc<dyn Room>, values: impl IntoIterator<Item = (i64, u64)>) -> Self {
        let mut s = Self { slots: Vector::new_in(Arc::clone(room)), len: 0, sq: 0, zero: 0 };
        s.slots.resize(SPARSE_FIRST, [NO_VALUE, 0]);
        for (v, n) in values.into_iter().filter(|&(_, n)| n > 0) {
            s.grow();
            let at = s.slot(v);
            if s.slots[at][0] == NO_VALUE {
                s.len += 1;
            }
            s.slots[at] = [v, n as i64];
            s.sq += n * n;
        }
        s
    }
    fn add_n(&mut self, v: i64, k: u64) -> bool {
        if !(0..LABEL_LIMIT).contains(&v) {
            return false;
        }
        if self.len >= SPARSE_LIMIT && self.slots[self.slot(v)][0] == NO_VALUE {
            return false;
        }
        self.grow();
        let at = self.slot(v);
        if self.slots[at][0] == NO_VALUE {
            self.slots[at] = [v, 0];
            self.len += 1;
        } else if self.slots[at][1] == 0 {
            // A value whose rows were all removed, back.
            self.zero -= 1;
        }
        let n = self.slots[at][1] as u64;
        self.sq += 2 * n * k + k * k;
        self.slots[at][1] += k as i64;
        true
    }
    /// Rows of `v` out: a value left with none keeps its slot at 0 (`zero` of them), as a removal from open
    /// addressing would break the probes past it.
    fn remove_n(&mut self, v: i64, k: u64) {
        if !(0..LABEL_LIMIT).contains(&v) {
            return;
        }
        let at = self.slot(v);
        let n = self.slots[at][1] as u64;
        let k = k.min(n);
        if self.slots[at][0] != v || k == 0 {
            return;
        }
        self.sq -= 2 * n * k - k * k;
        self.slots[at][1] -= k as i64;
        if n == k {
            self.zero += 1;
        }
    }
    fn count(&self, v: i64) -> Option<u64> {
        if !(0..LABEL_LIMIT).contains(&v) {
            return None;
        }
        let e = self.slots[self.slot(v)];
        Some(if e[0] == v { e[1] as u64 } else { 0 })
    }
    fn ndv(&self) -> usize {
        self.len - self.zero
    }
    fn counts(&self) -> impl Iterator<Item = (i64, u64)> + '_ {
        self.slots.iter().filter(|e| e[0] != NO_VALUE && e[1] > 0).map(|e| (e[0], e[1] as u64))
    }
}

/// One column's sketch (the module's): its registers, join counters and few values, its exact counts past them while
/// they stay bounded, and its rank counts past those. It is its arrays' one writer.
pub struct Sketch {
    registers: Vector<u8>,
    /// Its join-size counters and, while it has at most `FEW` values, each one's rows: this sketch is their one
    /// writer; the planner's join sizes and the statistics' keys read them there.
    counts: Arc<Counts>,
    /// Held by it and its clones while they share `counts`: a sketch that adds while another holds it writes a copy
    /// of its own first, the original as well as a clone, so neither's rows reach the other's counts (a clone's
    /// counts are the column's as it was cloned). The one left holding it alone writes the shared counts in place.
    sharing: Arc<()>,
    rows: u64,
    dense: Option<Dense>,
    sparse: Option<Sparse>,
    /// Once it keeps no exact counts, each register's rows at each rank (`REGISTERS` × `RANKS`, made from its last
    /// exact counts): a register is the highest rank with rows, so a value whose rows are all removed leaves it
    /// (`remove`). Its few values need none: their registers are made again from them. While it keeps a page or a
    /// label set, its distinct values are exact and a removed value stays in its registers, which only `overlap`
    /// reads then: it may count a key that is gone, never miss one that is there.
    ranks: Option<Vector<u32>>,
    /// Where its arrays are made, those it makes as it grows too.
    room: Arc<dyn Room>,
}

impl Default for Sketch {
    /// No rows, its arrays in the run's room.
    fn default() -> Sketch {
        Sketch::new_in(default_room())
    }
}

/// A clone shares the counts (`sharing`): whichever of them adds first while they are shared takes counts of its own,
/// so the other's few values are never ended in place under it.
impl Clone for Sketch {
    fn clone(&self) -> Sketch {
        Sketch { registers: self.registers.clone(), counts: Arc::clone(&self.counts), sharing: Arc::clone(&self.sharing), rows: self.rows, dense: self.dense.clone(), sparse: self.sparse.clone(), ranks: self.ranks.clone(), room: Arc::clone(&self.room) }
    }
}

/// Equal when they keep the same rows alike: the same exact counts, or past them the same rank counts and join counters
/// (in an exact form a removed value stays in the registers, so they are not compared there).
impl PartialEq for Sketch {
    fn eq(&self, other: &Sketch) -> bool {
        if self.rows != other.rows || !self.counts.counters().eq(other.counts.counters()) {
            return false;
        }
        match (self.exact_counts(), other.exact_counts()) {
            (Some(a), Some(b)) => {
                let (mut a, mut b) = (a.to_vec(), b.to_vec());
                a.sort_unstable();
                b.sort_unstable();
                a == b
            }
            (None, None) => match (&self.ranks, &other.ranks) {
                (Some(a), Some(b)) => a.iter().eq(b.iter()),
                _ => false,
            },
            _ => false,
        }
    }
}

impl Eq for Sketch {}

impl std::fmt::Debug for Sketch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let form = if self.counts.few_len().is_some() { "few" } else if self.dense.is_some() { "page" } else if self.sparse.is_some() { "labels" } else { "ranks" };
        f.debug_struct("Sketch").field("form", &form).field("rows", &self.rows).field("ndv", &self.ndv()).field("sq", &self.sq()).finish()
    }
}

/// What a sketch's state reads: its rows, distinct values and sum of squared rows a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
}

/// A version's change, signed: each value it touched, with its rows added less those taken (decision 0678 step 10).
/// Its size is the values the change touched, never the version's.
#[derive(Clone, Debug, Default)]
pub struct Delta {
    rows: i64,
    touched: FxHashMap<i64, i64>,
}

impl Delta {
    /// `n` rows of `v` added (`n` < 0: taken).
    pub fn add(&mut self, v: i64, n: i64) {
        self.rows += n;
        let e = self.touched.entry(v).or_insert(0);
        *e += n;
        if *e == 0 {
            self.touched.remove(&v);
        }
    }

    /// Its rows added less those taken.
    pub fn rows(&self) -> i64 {
        self.rows
    }

    /// The values it touched, each with its signed rows (none at 0).
    pub fn values(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.touched.iter().map(|(&v, &n)| (v, n))
    }

    pub fn is_empty(&self) -> bool {
        self.touched.is_empty()
    }

    pub fn len(&self) -> usize {
        self.touched.len()
    }
}

impl Sketch {
    /// No rows, its arrays (and those it makes as it grows) in `room`.
    pub fn new_in(room: Arc<dyn Room>) -> Sketch {
        Sketch { registers: zeros(&room, REGISTERS), counts: Arc::new(Counts::new_in(&room)), sharing: Arc::new(()), rows: 0, dense: None, sparse: None, ranks: None, room }
    }

    /// Counts of its own to write, where a clone shares them (`sharing`).
    fn own_counts(&mut self) {
        if Arc::strong_count(&self.sharing) > 1 {
            self.counts = Arc::new(Counts::clone(&self.counts));
            self.sharing = Arc::new(());
        }
    }

    /// A row of `v`. Returns the few values it held when this row ended them (the values with their rows, `v` among
    /// them): a writer that keeps a filter of the column's keys past its few values makes it from them.
    pub fn add(&mut self, v: i64) -> Option<Vec<i64>> {
        self.add_n(v, 1)
    }

    /// `n` rows of `v` (`add`'s, `n` at once).
    pub fn add_n(&mut self, v: i64, n: u64) -> Option<Vec<i64>> {
        if n == 0 {
            return None;
        }
        let h = hash(v);
        let (r, rho) = place(h);
        if rho > self.registers[r] {
            self.registers[r] = rho;
        }
        if let Some(ranks) = &mut self.ranks {
            ranks[cell(r, rho)] += n as u32;
        }
        self.own_counts();
        self.counts.count_n(h, n as i64);
        self.rows += n;
        if let Some(sparse) = &mut self.sparse {
            if !sparse.add_n(v, n) {
                let all: Vec<(i64, u64)> = sparse.counts().chain([(v, n)]).collect();
                self.start_ranks(all);
                self.sparse = None;
            }
        } else if let Some(dense) = &mut self.dense {
            if !dense.add_n(v, n) {
                if (0..LABEL_LIMIT).contains(&v) && dense.ndv < SPARSE_LIMIT {
                    let mut sparse = Sparse::from_counts(&self.room, dense.counts());
                    sparse.add_n(v, n);
                    self.sparse = Some(sparse);
                } else {
                    let all: Vec<(i64, u64)> = dense.counts().chain([(v, n)]).collect();
                    self.start_ranks(all);
                }
                self.dense = None;
            }
        } else if self.counts.few_len().is_some() && !self.counts.add_few_n(v, n as i64) {
            // Its `FEW + 1`th value: the few values and their rows, read where they lie (`counts`), make what
            // follows them.
            let counts = &self.counts;
            let small = || counts.few().into_iter().flatten();
            // Bounded label domains keep exact counts past the few values, without linear searches; large
            // identity columns go on to the rank counts.
            let base = v & !((DENSE_WIDTH as i64) - 1);
            if (0..LABEL_LIMIT).contains(&v) && small().all(|e| e[0] >= base && e[0] - base < DENSE_WIDTH as i64) {
                let mut dense = Dense { base, counts: zeros(&self.room, DENSE_WIDTH), ndv: counts.few_live().unwrap_or(0), sq: 0 };
                for [x, k] in small() {
                    dense.counts[(x - base) as usize] = k as u32;
                    dense.sq += (k as u64).pow(2);
                }
                dense.add_n(v, n);
                self.dense = Some(dense);
            } else if (0..LABEL_LIMIT).contains(&v) && small().all(|e| (0..LABEL_LIMIT).contains(&e[0])) {
                let mut sparse = Sparse::from_counts(&self.room, small().map(|e| (e[0], e[1] as u64)));
                sparse.add_n(v, n);
                self.sparse = Some(sparse);
            }
            // No exact counts left: its rows' ranks from here on, from the values it held and this one.
            if self.dense.is_none() && self.sparse.is_none() {
                let ranks = Self::ranks_of(&self.room, small().map(|e| (e[0], e[1] as u64)).chain([(v, n)]));
                for r in 0..REGISTERS {
                    self.registers[r] = Self::register(&ranks, r);
                }
                self.ranks = Some(ranks);
            }
            let ended: Vec<i64> = small().map(|e| e[0]).chain([v]).collect();
            self.counts.end_few();
            return Some(ended);
        }
        None
    }

    /// A row of value `v` taken out, as `add` put it in: in O(1), its few values' registers aside (made again from
    /// at most `FEW` values when one has no rows left). The row must be one it was given.
    pub fn remove(&mut self, v: i64) {
        self.remove_n(v, 1);
    }

    /// `n` rows of `v` taken out (`remove`'s, `n` at once).
    pub fn remove_n(&mut self, v: i64, n: u64) {
        let n = n.min(self.rows);
        if n == 0 {
            return;
        }
        let h = hash(v);
        let (r, rho) = place(h);
        self.own_counts();
        self.counts.count_n(h, -(n as i64));
        self.rows -= n;
        if let Some(sparse) = &mut self.sparse {
            sparse.remove_n(v, n);
        } else if let Some(dense) = &mut self.dense {
            dense.remove_n(v, n);
        } else if self.counts.remove_few_n(v, n as i64) == Some(0) && self.ranks.is_none() {
            // A value with no rows left: its few values' registers made again from those with rows.
            self.registers.iter_mut().for_each(|x| *x = 0);
            let live: Vec<i64> = self.counts.few().into_iter().flatten().filter(|e| e[1] > 0).map(|e| e[0]).collect();
            for x in live {
                let (r, rho) = place(hash(x));
                self.registers[r] = self.registers[r].max(rho);
            }
        }
        if let Some(ranks) = &mut self.ranks {
            let at = cell(r, rho);
            ranks[at] = ranks[at].saturating_sub(n as u32);
            if ranks[at] == 0 && usize::from(rho).min(RANKS) >= usize::from(self.registers[r]) {
                self.registers[r] = Self::register(ranks, r);
            }
        }
    }

    /// The rank counts of `values` (each with its rows).
    fn ranks_of(room: &Arc<dyn Room>, values: impl Iterator<Item = (i64, u64)>) -> Vector<u32> {
        let mut ranks: Vector<u32> = zeros(room, REGISTERS * RANKS);
        for (x, n) in values {
            let (r, rho) = place(hash(x));
            ranks[cell(r, rho)] += n as u32;
        }
        ranks
    }

    /// Its rank counts made as it ends its last exact counts, from them (`values`: each with its rows, the row being
    /// added among them), and its registers from those.
    fn start_ranks(&mut self, values: Vec<(i64, u64)>) {
        let ranks = Self::ranks_of(&self.room, values.into_iter());
        for r in 0..REGISTERS {
            self.registers[r] = Self::register(&ranks, r);
        }
        self.ranks = Some(ranks);
    }

    /// Register `r` from its rank counts: its highest rank with rows, 0 for none.
    fn register(ranks: &[u32], r: usize) -> u8 {
        ranks[r * RANKS..(r + 1) * RANKS].iter().rposition(|&n| n > 0).map_or(0, |i| i as u8 + 1)
    }

    /// Its rows.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Distinct values.
    pub fn ndv(&self) -> usize {
        if let Some(s) = &self.sparse {
            return s.ndv();
        }
        if let Some(d) = &self.dense {
            return d.ndv;
        }
        if let Some(n) = self.counts.few_live() {
            return n;
        }
        Self::estimate(self.registers.iter().copied(), self.rows)
    }

    /// Estimated fraction of the probe column's distinct keys present in the build column. Inclusion-exclusion can
    /// lose a small intersection in the sketch error. This only prices a plan.
    pub fn overlap(&self, other: &Self) -> f64 {
        if self.rows == 0 || other.rows == 0 {
            return 0.0;
        }
        // The three estimates alike, none clamped to its rows: a side's estimate clamped where the union's is not
        // made a sketch overlap itself less than whole when its estimate ran over its rows.
        let left = Self::estimate(self.registers.iter().copied(), u64::MAX);
        let right = Self::estimate(other.registers.iter().copied(), u64::MAX);
        let union = Self::estimate(self.registers.iter().zip(other.registers.iter()).map(|(&a, &b)| a.max(b)), u64::MAX);
        (left.saturating_add(right).saturating_sub(union).min(left.min(right)) as f64) / left.max(1) as f64
    }

    fn estimate(registers: impl Iterator<Item = u8>, rows: u64) -> usize {
        if rows == 0 {
            return 0;
        }
        let m = REGISTERS as f64;
        let mut sum = 0.0;
        let mut zeros = 0;
        for rank in registers {
            sum += 2f64.powi(-i32::from(rank));
            zeros += usize::from(rank == 0);
        }
        let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
        let est = if raw <= 2.5 * m && zeros > 0 { m * (m / zeros as f64).ln() } else { raw };
        (est.round() as usize).clamp(1, usize::try_from(rows).unwrap_or(usize::MAX))
    }

    /// The sum of squares from join counters, at least a row a value and at most every row one value.
    fn sq_of(counters: impl Iterator<Item = i64>, rows: u64) -> u64 {
        let est: f64 = counters.map(|c| (c as f64) * (c as f64)).sum();
        (est.round() as u64).clamp(rows, rows.saturating_mul(rows))
    }

    /// The sum over values of each value's rows squared.
    pub fn sq(&self) -> u64 {
        if let Some(s) = &self.sparse {
            return s.sq;
        }
        if let Some(d) = &self.dense {
            return d.sq;
        }
        if let Some(few) = self.counts.few() {
            return few.map(|e| (e[1] as u64) * (e[1] as u64)).sum();
        }
        Self::sq_of(self.counts.counters().map(i64::from), self.rows)
    }

    /// Whether it keeps each value's rows exactly (its few values, a page or a label set).
    pub fn is_exact(&self) -> bool {
        self.sparse.is_some() || self.dense.is_some() || self.counts.few_len().is_some()
    }

    /// Value `v`'s rows, while it keeps them exactly.
    pub fn count(&self, v: i64) -> Option<u64> {
        if let Some(s) = &self.sparse {
            return s.count(v);
        }
        if let Some(d) = &self.dense {
            return d.count(v);
        }
        Some(self.counts.few()?.find(|e| e[0] == v).map_or(0, |e| e[1].max(0) as u64))
    }

    /// Each value's rows, while they are exact: read from the sketch's arrays.
    pub fn exact_counts(&self) -> Option<Vector<[i64; 2]>> {
        if let Some(s) = self.counts.few() {
            return Some(s.filter(|e| e[1] > 0).collect());
        }
        if let Some(s) = &self.sparse {
            return Some(s.counts().map(|(v, n)| [v, n as i64]).collect());
        }
        self.dense.as_ref().map(|d| d.counts().map(|(v, n)| [v, n as i64]).collect())
    }

    /// Its join counts, read where they lie (the planner's join sizes and the statistics' keys hold them).
    pub fn counts(&self) -> &Arc<Counts> {
        &self.counts
    }

    /// Its state read with `d`, its version's change since (decision 0678 step 10), in `d`'s size: while it keeps
    /// exact counts, each touched value's rows before and after; past them, each touched register made again from
    /// its rank counts and the change's, and the join counters moved by the change's values. The same reading as a
    /// sketch of the state's rows, within the sketch's error where it keeps no exact counts.
    pub fn with(&self, d: &Delta) -> Reading {
        let rows = (self.rows as i64 + d.rows).max(0) as u64;
        if self.is_exact() {
            let (mut ndv, mut sq) = (self.ndv() as i64, self.sq() as i64);
            for (v, n) in d.values() {
                let a = self.count(v).unwrap_or(0) as i64;
                let b = (a + n).max(0);
                ndv += i64::from(b > 0) - i64::from(a > 0);
                sq += b * b - a * a;
            }
            return Reading { rows, ndv: ndv.max(0) as u64, sq: sq.max(0) as u64 };
        }
        let ranks = self.ranks.as_ref().expect("past its exact counts, its rank counts");
        let mut cells: FxHashMap<usize, i64> = FxHashMap::default();
        let mut counters: Vec<i64> = self.counts.counters().map(i64::from).collect();
        for (v, n) in d.values() {
            let h = hash(v);
            let (r, rho) = place(h);
            *cells.entry(cell(r, rho)).or_insert(0) += n;
            let (b, s) = bucket(h);
            counters[b] += i64::from(s) * n;
        }
        let mut registers: Vec<u8> = self.registers.to_vec();
        let touched: std::collections::BTreeSet<usize> = cells.keys().map(|c| c / RANKS).collect();
        for r in touched {
            registers[r] = (0..RANKS).rev().find(|&i| i64::from(ranks[r * RANKS + i]) + cells.get(&(r * RANKS + i)).copied().unwrap_or(0) > 0).map_or(0, |i| i as u8 + 1);
        }
        let ndv = Self::estimate(registers.into_iter(), rows) as u64;
        Reading { rows, ndv, sq: Self::sq_of(counters.into_iter(), rows) }
    }

    /// Its state read alone.
    pub fn reading(&self) -> Reading {
        Reading { rows: self.rows, ndv: self.ndv() as u64, sq: self.sq() }
    }

    /// `d` folded in: its values' rows added and taken, in `d`'s size. Returns the few values it held when an added
    /// value ended them (`add`'s).
    pub fn apply(&mut self, d: &Delta) -> Option<Vec<i64>> {
        let mut ended = None;
        for (v, n) in d.values() {
            if n > 0 {
                ended = self.add_n(v, n as u64).or(ended);
            } else {
                self.remove_n(v, n.unsigned_abs());
            }
        }
        ended
    }

    /// `other`'s rows added: the sketch of both (each part is linear). In `other`'s exact counts' size where it keeps
    /// them; else this one goes past its own exact counts too, and `other`'s rank counts and counters are added cell by
    /// cell. Returns the few values it held when this ended them (`add`'s).
    pub fn merge(&mut self, other: &Sketch) -> Option<Vec<i64>> {
        if other.rows == 0 {
            return None;
        }
        if let Some(values) = other.exact_counts() {
            let mut ended = None;
            for &[v, n] in values.iter() {
                ended = self.add_n(v, n as u64).or(ended);
            }
            return ended;
        }
        let theirs = other.ranks.as_ref().expect("past its exact counts, its rank counts");
        self.own_counts();
        let mut ended = None;
        if self.ranks.is_none() {
            let held: Vec<(i64, u64)> = self.exact_counts().map(|v| v.iter().map(|e| (e[0], e[1] as u64)).collect()).unwrap_or_default();
            if let Some(few) = self.counts.few() {
                ended = Some(few.map(|e| e[0]).collect());
            }
            self.start_ranks(held);
            self.dense = None;
            self.sparse = None;
            self.counts.end_few();
        }
        let ranks = self.ranks.as_mut().expect("made");
        for (c, &n) in theirs.iter().enumerate() {
            ranks[c] += n;
        }
        for r in 0..REGISTERS {
            self.registers[r] = Self::register(ranks, r);
        }
        self.counts.add_counters(&other.counts);
        self.rows += other.rows;
        ended
    }

    /// Its bytes, as a store keeps them: its form, rows, registers and join counters, then how many entries follow and
    /// the entries: its exact counts (few values, a page after its base, or a label set: the values with rows) or, past
    /// them, its rank counts that are not 0. Varints
    /// throughout, so a column of few values or rows takes few bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![SKETCH_FORMAT];
        let form = if self.counts.few_len().is_some() { 0 } else if self.dense.is_some() { 1 } else if self.sparse.is_some() { 2 } else { 3 };
        out.push(form);
        varint(&mut out, self.rows);
        // copy: derived (the sketch's kept form, written once as the store keeps it; `decode` reads it back)
        out.extend_from_slice(&self.registers);
        for c in self.counts.counters() {
            varint(&mut out, zigzag(i64::from(c)));
        }
        match form {
            0 => {
                let few: Vec<[i64; 2]> = self.counts.few().into_iter().flatten().collect();
                varint(&mut out, few.len() as u64);
                for [v, n] in few {
                    varint(&mut out, zigzag(v));
                    varint(&mut out, n as u64);
                }
            }
            1 => {
                let d = self.dense.as_ref().expect("a page");
                let live: Vec<(usize, u32)> = d.counts.iter().enumerate().filter(|e| *e.1 > 0).map(|(i, &n)| (i, n)).collect();
                varint(&mut out, live.len() as u64);
                varint(&mut out, zigzag(d.base));
                let mut last = 0;
                for (i, n) in live {
                    varint(&mut out, (i - last) as u64);
                    varint(&mut out, u64::from(n));
                    last = i;
                }
            }
            2 => {
                let live: Vec<(i64, u64)> = self.sparse.as_ref().expect("a label set").counts().collect();
                varint(&mut out, live.len() as u64);
                for (v, n) in live {
                    varint(&mut out, v as u64);
                    varint(&mut out, n);
                }
            }
            _ => {
                let ranks = self.ranks.as_ref().expect("rank counts");
                let live: Vec<(usize, u32)> = ranks.iter().enumerate().filter(|e| *e.1 > 0).map(|(i, &n)| (i, n)).collect();
                varint(&mut out, live.len() as u64);
                let mut last = 0;
                for (i, n) in live {
                    varint(&mut out, (i - last) as u64);
                    varint(&mut out, u64::from(n));
                    last = i;
                }
            }
        }
        out
    }

    /// `encode`'s bytes `from..from + n` read back where they lie (`byte`), into a sketch whose arrays are in `room`:
    /// `None` when they are not a sketch's of this format.
    pub fn decode(byte: &dyn Fn(usize) -> u8, from: usize, n: usize, room: Arc<dyn Room>) -> Option<Sketch> {
        let end = from.checked_add(n)?;
        let mut at = from;
        let mut next = || -> Option<u8> {
            (at < end).then(|| {
                at += 1;
                byte(at - 1)
            })
        };
        if next()? != SKETCH_FORMAT {
            return None;
        }
        let form = next()?;
        let mut s = Sketch::new_in(room);
        s.rows = read_varint(&mut next)?;
        for r in 0..REGISTERS {
            s.registers[r] = next()?;
        }
        for b in 0..BUCKETS {
            s.counts.counter(b).store(unzigzag(read_varint(&mut next)?) as i32, Relaxed);
        }
        let count = read_varint(&mut next)? as usize;
        match form {
            0 => {
                if count > FEW {
                    return None;
                }
                for _ in 0..count {
                    let v = unzigzag(read_varint(&mut next)?);
                    let k = read_varint(&mut next)? as i64;
                    let e = s.counts.few_len()?;
                    let cell = s.counts.few_cell(e);
                    cell[0].store(v, Relaxed);
                    cell[1].store(k, Relaxed);
                    s.counts.len.store(e as u32 + 1, std::sync::atomic::Ordering::Release);
                }
            }
            1 => {
                let base = unzigzag(read_varint(&mut next)?);
                let mut d = Dense { base, counts: zeros(&s.room, DENSE_WIDTH), ndv: 0, sq: 0 };
                let mut i = 0usize;
                for e in 0..count {
                    let step = read_varint(&mut next)? as usize;
                    i += step;
                    // Each index past the last: a step of 0 after the first is a repeated one.
                    if e > 0 && step == 0 || i >= DENSE_WIDTH {
                        return None;
                    }
                    let k = read_varint(&mut next)?;
                    d.counts[i] = k as u32;
                    d.ndv += 1;
                    d.sq += k * k;
                }
                s.counts.end_few();
                s.dense = Some(d);
            }
            2 => {
                let mut values = Vec::with_capacity(count.min(SPARSE_LIMIT));
                for _ in 0..count {
                    let v = read_varint(&mut next)? as i64;
                    values.push((v, read_varint(&mut next)?));
                }
                if count > SPARSE_LIMIT || values.iter().any(|&(v, _)| !(0..LABEL_LIMIT).contains(&v)) {
                    return None;
                }
                s.counts.end_few();
                s.sparse = Some(Sparse::from_counts(&s.room, values));
            }
            3 => {
                let mut ranks: Vector<u32> = zeros(&s.room, REGISTERS * RANKS);
                let mut i = 0usize;
                for e in 0..count {
                    let step = read_varint(&mut next)? as usize;
                    i += step;
                    if e > 0 && step == 0 {
                        return None;
                    }
                    *ranks.get_mut(i)? = read_varint(&mut next)? as u32;
                }
                s.counts.end_few();
                s.ranks = Some(ranks);
            }
            _ => return None,
        }
        (at == end).then_some(s)
    }

    /// The sketch of `values` (each with its rows; none at 0 or below), its arrays in `room`.
    pub fn from_counts(room: Arc<dyn Room>, values: impl IntoIterator<Item = (i64, i64)>) -> Sketch {
        let mut s = Sketch::new_in(room);
        for (v, n) in values {
            if n > 0 {
                s.add_n(v, n as u64);
            }
        }
        s
    }

    /// The same rows with no value kept: past its exact counts, its rank counts made from them, so it reads as the
    /// sketch of those rows, within its error, and holds none of their values. A writer that keeps the exact counts
    /// apart (their values mapped as they are written) writes this in their place.
    pub fn without_values(&self) -> Sketch {
        let mut s = self.clone();
        if let Some(held) = self.exact_counts() {
            s.own_counts();
            s.start_ranks(held.iter().map(|e| (e[0], e[1] as u64)).collect());
            s.dense = None;
            s.sparse = None;
            s.counts.end_few();
        }
        s
    }

    /// Whether `encode`'s bytes at `from` (`byte`) are of a sketch that keeps exact counts, read without decoding them.
    pub fn kept_exact(byte: &dyn Fn(usize) -> u8, from: usize) -> bool {
        byte(from) == SKETCH_FORMAT && byte(from + 1) < 3
    }

    /// Its registers (for `overlap`, and a reader's own estimate).
    pub fn registers(&self) -> &[u8] {
        &self.registers
    }

    /// Its arrays' bytes.
    pub fn bytes(&self) -> usize {
        self.sparse.as_ref().map_or(0, |s| s.slots.len() * 16) + self.dense.as_ref().map_or(0, |_| DENSE_WIDTH * 4) + REGISTERS + self.ranks.as_ref().map_or(0, |r| r.len() * 4) + self.counts.bytes()
    }

    /// Whether its arrays are in blocks (decision 0478 step 6c).
    pub fn in_blocks(&self) -> bool {
        use crate::maps::Placed;
        self.registers.place().is_some() && self.counts.in_blocks() && self.dense.as_ref().is_none_or(|d| d.counts.place().is_some()) && self.sparse.as_ref().is_none_or(|s| s.slots.place().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of the tests' own: this crate's tests install no run room, so `Sketch::default` takes the heap's.
    fn room() -> Arc<dyn Room> {
        Arc::new(crate::maps::InBlock(Arc::new(crate::block::Block::anon(crate::block::kind::FACTS, 1 << 22).with_free_list())))
    }

    fn of(values: impl IntoIterator<Item = i64>) -> Sketch {
        let mut s = Sketch::new_in(room());
        for v in values {
            s.add(v);
        }
        s
    }

    /// The forms a column's sketch takes, by its values: few, a page, a label set, and many (rank counts).
    fn forms() -> Vec<(&'static str, Vec<i64>)> {
        vec![
            ("few values", (0..30).map(|i| i % 10).collect()),
            ("a page", (0..600).map(|i| i % 200).collect()),
            ("labels", (0..600).map(|i| (i % 200) * 293).collect()),
            ("many", (0..60_000i64).map(|i| (i % 20_000) * 7_919 + (1 << 40)).collect()),
        ]
    }

    #[test]
    fn sketches_estimate_distinct_values_and_squares() {
        let few = of((0..1000).map(|i| i % 10));
        assert_eq!((few.ndv(), few.sq()), (10, 10 * 100 * 100));
        let many = of((0..200_000i64).map(|i| i % 50_000));
        let ndv = many.ndv() as f64;
        assert!((ndv / 50_000.0 - 1.0).abs() < 0.2, "{ndv}");
        // 50,000 values of 4 rows: 800,000.
        let sq = many.sq() as f64;
        assert!((sq / 800_000.0 - 1.0).abs() < 0.35, "{sq}");
        let skewed = of((0..100_000i64).map(|i| if i % 2 == 0 { 7 } else { i }));
        // One value of 50,000 rows dominates: about 2.5e9.
        let sq = skewed.sq() as f64;
        assert!((sq / 2.5e9 - 1.0).abs() < 0.2, "{sq}");
    }

    /// A value's rows added at once are its rows added one by one: every part alike.
    #[test]
    fn rows_added_at_once_are_rows_added_one_by_one() {
        for (what, all) in forms() {
            let mut counted: Vec<(i64, u64)> = Vec::new();
            for &v in &all {
                match counted.iter_mut().find(|e| e.0 == v) {
                    Some(e) => e.1 += 1,
                    None => counted.push((v, 1)),
                }
            }
            let mut at_once = Sketch::new_in(room());
            for &(v, n) in &counted {
                at_once.add_n(v, n);
            }
            let mut sorted = all.clone();
            sorted.sort_by_key(|&v| counted.iter().position(|e| e.0 == v));
            let one_by_one = of(sorted);
            assert_eq!(at_once.reading(), one_by_one.reading(), "{what}");
            assert_eq!(at_once.registers(), one_by_one.registers(), "{what}");
            assert_eq!(at_once.counts.counters().collect::<Vec<_>>(), one_by_one.counts.counters().collect::<Vec<_>>(), "{what}");
        }
    }

    /// Decision 0678 step 10: a version's sketch read with its delta reads as a sketch of the state's rows, in each
    /// form, and the delta folded in leaves the sketch as that one's: rows added, rows taken, a value taken whole and
    /// one brought back.
    #[test]
    fn a_version_read_with_its_delta_is_the_states_sketch() {
        for (what, all) in forms() {
            let version = of(all.iter().copied());
            // The change: every fifth row gone, one value's every row gone, new rows of values seen and unseen.
            let whole = all[1];
            let gone: Vec<i64> = all.iter().enumerate().filter(|&(i, &v)| i % 5 == 0 || v == whole).map(|(_, &v)| v).collect();
            let fresh: Vec<i64> = (0..40).map(|i| if i % 2 == 0 { all[i * 7 % all.len()] } else { all[2] + i as i64 * 3 }).collect();
            let mut d = Delta::default();
            for &v in &gone {
                d.add(v, -1);
            }
            for &v in &fresh {
                d.add(v, 1);
            }
            let mut state: Vec<i64> = all.clone();
            for v in &gone {
                let at = state.iter().position(|x| x == v).unwrap();
                state.swap_remove(at);
            }
            state.extend(&fresh);
            let want = of(state.iter().copied());
            let got = version.with(&d);
            if version.is_exact() {
                assert_eq!(got, want.reading(), "{what}: read with its delta");
            } else {
                assert_eq!(got.rows, want.rows(), "{what}");
                // Past exact counts both are the registers' and counters' estimates, from equal registers and
                // counters (the state's own sketch holds no exact counts either).
                assert!(!want.is_exact());
                assert_eq!((got.ndv, got.sq), (want.ndv() as u64, want.sq()), "{what}: read with its delta");
            }
            let mut folded = version.clone();
            folded.apply(&d);
            assert_eq!(folded.rows(), want.rows(), "{what}");
            assert_eq!(folded.counts.counters().collect::<Vec<_>>(), want.counts.counters().collect::<Vec<_>>(), "{what}: join counters");
            if !version.is_exact() {
                assert_eq!(folded.registers(), want.registers(), "{what}: registers");
            }
            assert_eq!(folded.reading(), got, "{what}: folded, it reads as it read with the delta");
        }
    }

    /// A delta's size is the values it touched: a value added and taken again leaves nothing.
    #[test]
    fn a_delta_holds_only_the_values_it_changed() {
        let mut d = Delta::default();
        for v in 0..100 {
            d.add(v, 1);
        }
        for v in 0..90 {
            d.add(v, -1);
        }
        assert_eq!((d.len(), d.rows()), (10, 10));
        assert_eq!(of(0..5).with(&Delta::default()), of(0..5).reading());
    }

    /// Decision 0478 step 6c: a sketch's arrays (registers, counters, exact counts in each form) are block arrays.
    #[test]
    fn a_sketchs_arrays_are_in_blocks() {
        for (what, all) in forms() {
            assert!(of(all).in_blocks(), "{what}");
        }
    }

    /// Two sketches merged are the sketch of both columns' rows, in each pair of forms: exact counts alike where both
    /// keep them; past them, the same rank counts, registers and join counters as a sketch of all the rows.
    #[test]
    fn two_sketches_merged_are_the_sketch_of_both() {
        let same = |a: &Sketch, b: &Sketch| a.rows() == b.rows() && a.registers() == b.registers() && a.counts().counters().eq(b.counts().counters());
        let exact = |s: &Sketch| s.exact_counts().map(|v| { let mut v: Vec<[i64; 2]> = v.to_vec(); v.sort(); v });
        for (a_what, a) in forms() {
            for (b_what, b) in forms() {
                let what = format!("{a_what} + {b_what}");
                let mut merged = of(a.iter().copied());
                merged.merge(&of(b.iter().copied()));
                let both = of(a.iter().chain(b.iter()).copied());
                assert_eq!(merged.reading().rows, both.reading().rows, "{what}");
                assert_eq!(merged.counts().counters().collect::<Vec<_>>(), both.counts().counters().collect::<Vec<_>>(), "{what}: join counters");
                if both.is_exact() && merged.is_exact() {
                    assert_eq!((exact(&merged), merged.ndv(), merged.sq()), (exact(&both), both.ndv(), both.sq()), "{what}: exact counts");
                } else if !merged.is_exact() && !both.is_exact() {
                    assert!(same(&merged, &both), "{what}: rows, registers, counters");
                    assert_eq!((merged.ndv(), merged.sq()), (both.ndv(), both.sq()), "{what}");
                } else {
                    // One went past its exact counts where the other did not (a merge of many and few values may end
                    // them earlier): the estimates agree within the sketch's error.
                    let (m, w) = (merged.ndv() as f64, both.ndv() as f64);
                    assert!((m / w - 1.0).abs() < 0.2, "{what}: {m} against {w}");
                }
            }
        }
    }

    /// A sketch without its values reads as the sketch of its rows (within the estimate's error) and holds none of them;
    /// a sketch made from its exact counts is the sketch; equality compares the rows kept, not the order they came in.
    #[test]
    fn a_sketch_without_its_values_reads_as_its_rows() {
        let values: Vec<i64> = (0..300).map(|i| i % 90).collect();
        let s = {
            let mut s = Sketch::new_in(room());
            values.iter().for_each(|&v| drop(s.add(v)));
            s
        };
        let counts: Vec<(i64, i64)> = s.exact_counts().expect("exact").iter().map(|e| (e[0], e[1])).collect();
        let made = Sketch::from_counts(room(), counts.iter().rev().copied());
        assert_eq!(made, s, "made from its counts, in another order");
        let bare = s.without_values();
        assert!(!bare.is_exact() && bare.exact_counts().is_none(), "no values held");
        assert_eq!(bare.rows(), s.rows());
        assert!((bare.ndv() as f64 - 90.0).abs() < 90.0 * 0.2, "about its distinct values: {}", bare.ndv());
        let bytes = bare.encode();
        assert!(!Sketch::kept_exact(&|i| bytes[i], 0));
        let bytes = s.encode();
        assert!(Sketch::kept_exact(&|i| bytes[i], 0));
        assert_ne!(bare, s);
        assert!(s.is_exact(), "the sketch itself unchanged");
    }

    /// A sketch read back from its bytes is the sketch it was, in each form: its bytes alike, its readings alike,
    /// and rows added and taken after alike.
    #[test]
    fn a_sketch_read_back_from_its_bytes_is_the_sketch() {
        for (what, all) in forms() {
            let mut s = of(all.iter().copied());
            s.remove(all[0]);
            let bytes = s.encode();
            let mut back = Sketch::decode(&|i| bytes[i], 0, bytes.len(), room()).expect(&what);
            assert_eq!(back.encode(), bytes, "{what}: bytes");
            assert_eq!(back.reading(), s.reading(), "{what}: reading");
            assert!(back.in_blocks(), "{what}: in blocks");
            for v in [all[1], all[2], 1 << 41] {
                s.add(v);
                back.add(v);
            }
            s.remove(all[3]);
            back.remove(all[3]);
            assert_eq!((back.encode(), back.reading()), (s.encode(), s.reading()), "{what}: after");
            assert!(Sketch::decode(&|i| bytes[i], 0, bytes.len() - 1, room()).is_none(), "{what}: cut short");
        }
    }

}
