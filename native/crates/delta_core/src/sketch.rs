//! A column's kept sketch read where it lies (decision 0678 step 10; the format is `rerail_blocks::sketch`'s, whose
//! `Sketch` adds, takes and encodes): its reading (rows, distinct values, the sum of each value's rows squared), taken
//! from the encoded bytes as they stream, so a trace's statistics read nothing into a structure of their own.
//!
//! The bytes, as `Sketch::encode` writes them: the format, the form (0 few values, 1 a page, 2 a label set, 3 rank
//! counts), the rows, `REGISTERS` registers, `BUCKETS` join counters, then how many entries follow and the entries.
//! Varints throughout, the signed ones zigzagged.

use crate::refs::Span;
use crate::relation::Relation;

/// What a sketch's state reads: its rows, distinct values and sum of squared rows a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
}

/// The join counters a column keeps.
pub const BUCKETS: usize = 256;
/// The values a column keeps its few exact counts of, at most.
pub const FEW: usize = 64;
pub const REGISTERS: usize = 256;
/// The ranks a register's rows are counted at.
pub const RANKS: usize = 32;
pub const DENSE_WIDTH: usize = 4096;
pub const LABEL_LIMIT: i64 = 1_i64 << 32;
pub const SPARSE_LIMIT: usize = 1024;
/// The kept form's format.
pub const SKETCH_FORMAT: u8 = 1;

pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// A varint from `next`: `None` past the bytes or past 64 bits.
pub fn read_varint(next: &mut impl FnMut() -> Option<u8>) -> Option<u64> {
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

/// Column `c`'s kept sketch in a layer's relation (its sketches' directory, `Span<u8>`s in its block): `None` when it
/// keeps none.
pub fn kept(rel: &Relation, c: usize) -> Option<&[u8]> {
    let b = rel.sketches.as_slice();
    if b.is_empty() {
        return None;
    }
    // SAFETY: its writer laid the directory out as `Span<u8>`s, aligned to 8, in the relation's block, frozen since.
    let dir = unsafe { core::slice::from_raw_parts(b.as_ptr().cast::<Span<u8>>(), b.len() / size_of::<Span<u8>>()) };
    dir.get(c).filter(|s| !s.is_empty()).map(Span::as_slice)
}

/// `2^-rank`, exactly (a power of two's bits).
fn pow2_neg(rank: u8) -> f64 {
    f64::from_bits((1023 - u64::from(rank)) << 52)
}

/// `ln x` for `x >= 1`: its power of two's, then `2 atanh((f - 1) / (f + 1))` of the rest (`f` in `[1, 2)`), within an
/// ulp or two of the platform's. (`core` has no `ln`; the floor links no math library.)
fn ln(x: f64) -> f64 {
    debug_assert!(x >= 1.0 && x.is_finite());
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let f = f64::from_bits((bits & ((1u64 << 52) - 1)) | (1023u64 << 52));
    let s = (f - 1.0) / (f + 1.0);
    let (s2, mut term, mut sum) = (s * s, s, 0.0);
    for k in 0..30 {
        sum += term / f64::from(2 * k + 1);
        term *= s2;
    }
    e as f64 * core::f64::consts::LN_2 + 2.0 * sum
}

/// `x` rounded half away from zero, for `0 <= x < 2^63`.
fn round_pos(x: f64) -> u64 {
    let t = x as u64;
    if x - t as f64 >= 0.5 { t + 1 } else { t }
}

/// Distinct values from the registers (HyperLogLog, with its small-range correction), at least 1 and at most `rows`.
fn estimate(registers: &[u8; REGISTERS], rows: u64) -> u64 {
    if rows == 0 {
        return 0;
    }
    let m = REGISTERS as f64;
    let (mut sum, mut zeros) = (0.0, 0u32);
    for &rank in registers {
        sum += pow2_neg(rank);
        zeros += u32::from(rank == 0);
    }
    let raw = 0.7213 / (1.0 + 1.079 / m) * m * m / sum;
    let est = if raw <= 2.5 * m && zeros > 0 { m * ln(m / f64::from(zeros)) } else { raw };
    round_pos(est).clamp(1, rows)
}

/// The sum of squares from join counters, at least a row a value and at most every row one value.
fn sq_of(counters: &[i32; BUCKETS], rows: u64) -> u64 {
    let est: f64 = counters.iter().map(|&c| f64::from(c) * f64::from(c)).sum();
    round_pos(est).clamp(rows, rows.saturating_mul(rows))
}

/// The reading of a kept sketch's bytes (`Sketch::encode`'s), as its decoded sketch reads: `None` when they are not a
/// sketch's of this format, by `Sketch::decode`'s checks.
pub fn reading(bytes: &[u8]) -> Option<Reading> {
    let mut at = 0usize;
    let mut next = || -> Option<u8> {
        let b = *bytes.get(at)?;
        at += 1;
        Some(b)
    };
    if next()? != SKETCH_FORMAT {
        return None;
    }
    let form = next()?;
    let rows = read_varint(&mut next)?;
    let mut registers = [0u8; REGISTERS];
    for r in registers.iter_mut() {
        *r = next()?;
    }
    let mut counters = [0i32; BUCKETS];
    for c in counters.iter_mut() {
        *c = unzigzag(read_varint(&mut next)?) as i32;
    }
    let count = read_varint(&mut next)? as usize;
    let (ndv, sq) = match form {
        0 => {
            if count > FEW {
                return None;
            }
            let (mut ndv, mut sq) = (0u64, 0u64);
            for _ in 0..count {
                unzigzag(read_varint(&mut next)?);
                let k = read_varint(&mut next)? as i64;
                ndv += u64::from(k > 0);
                sq = sq.wrapping_add((k as u64).wrapping_mul(k as u64));
            }
            (ndv, sq)
        }
        1 => {
            unzigzag(read_varint(&mut next)?);
            let (mut i, mut sq) = (0usize, 0u64);
            for e in 0..count {
                let step = read_varint(&mut next)? as usize;
                i = i.checked_add(step)?;
                if e > 0 && step == 0 || i >= DENSE_WIDTH {
                    return None;
                }
                // The page keeps it as a u32; its sum of squares, as the decoder adds it, the count read.
                let k = read_varint(&mut next)?;
                sq += k * k;
            }
            (count as u64, sq)
        }
        2 => {
            let (mut ndv, mut sq) = (0u64, 0u64);
            for _ in 0..count {
                let v = read_varint(&mut next)? as i64;
                let n = read_varint(&mut next)?;
                if !(0..LABEL_LIMIT).contains(&v) {
                    return None;
                }
                if n > 0 {
                    ndv += 1;
                    sq += n * n;
                }
            }
            if count > SPARSE_LIMIT {
                return None;
            }
            (ndv, sq)
        }
        3 => {
            let mut i = 0usize;
            for e in 0..count {
                let step = read_varint(&mut next)? as usize;
                i = i.checked_add(step)?;
                if e > 0 && step == 0 || i >= REGISTERS * RANKS {
                    return None;
                }
                read_varint(&mut next)?;
            }
            (estimate(&registers, rows), sq_of(&counters, rows))
        }
        _ => return None,
    };
    (at == bytes.len()).then_some(Reading { rows, ndv, sq })
}

// ---- a sketch worked where its bytes lie (decision 0686: an operation's statistics, with no heap) ------------------

/// Slots of a label set at most, and at first (`rerail_blocks::sketch`'s `Sparse`: it doubles while its values pass
/// half its slots).
pub const SPARSE_SLOTS: usize = 2 * SPARSE_LIMIT;
pub const SPARSE_FIRST: usize = 128;
/// An empty slot's value.
const NO_VALUE: i64 = -1;
/// The few values' length once they are ended.
const PAST: u64 = u64::MAX;

const FORM_FEW: u64 = 0;
const FORM_PAGE: u64 = 1;
const FORM_LABELS: u64 = 2;
const FORM_RANKS: u64 = 3;

/// One column's sketch as its writer works it, in fixed memory (an operation's scratch, `SketchW::over`): the same
/// parts and forms as `rerail_blocks::sketch::Sketch`, each at its bound, so a phase that makes a layer's statistics
/// takes no heap and runs as a device would. Its kept bytes are `Sketch::encode`'s, byte for byte (the blocks crate's
/// tests check the two alike, add by add).
#[repr(C)]
pub struct Work {
    rows: u64,
    form: u64,
    few_len: u64,
    page_base: i64,
    page_ndv: u64,
    page_sq: u64,
    label_cap: u64,
    label_len: u64,
    label_sq: u64,
    label_zero: u64,
    registers: [u8; REGISTERS],
    counters: [i32; BUCKETS],
    few: [[i64; 2]; FEW],
    page: [u32; DENSE_WIDTH],
    labels: [[i64; 2]; SPARSE_SLOTS],
    ranks: [u32; REGISTERS * RANKS],
}

/// The bytes a working sketch takes (`SketchW::over`), aligned to 8.
pub const WORK_BYTES: usize = size_of::<Work>();

#[inline(always)]
fn bucket(h: u64) -> (usize, i32) {
    (((h >> 8) & 0xff) as usize, if h & 1 == 0 { 1 } else { -1 })
}

#[inline(always)]
fn place(h: u64) -> (usize, u8) {
    ((h >> 56) as usize, ((h << 8) | 0x80).leading_zeros() as u8 + 1)
}

#[inline(always)]
fn cell(r: usize, rho: u8) -> usize {
    r * RANKS + usize::from(rho).min(RANKS) - 1
}

fn labelled(v: i64) -> bool {
    (0..LABEL_LIMIT).contains(&v)
}

/// A sketch being worked: its parts in a `Work` its caller lends.
pub struct SketchW<'a> {
    w: &'a mut Work,
}

impl<'a> SketchW<'a> {
    /// A working sketch over `bytes` (at least `WORK_BYTES`, aligned to 8), with no rows: `None` when they are too
    /// few or misaligned. What they held is not read: each part is written before it is read.
    pub fn over(bytes: &'a mut [u8]) -> Option<SketchW<'a>> {
        if bytes.len() < WORK_BYTES || !(bytes.as_ptr() as usize).is_multiple_of(align_of::<Work>()) {
            return None;
        }
        // SAFETY: `WORK_BYTES` bytes of the caller's, aligned for `Work`, held for `'a`; every field is plain integers,
        // so any bytes are a value of it.
        let w = unsafe { &mut *bytes.as_mut_ptr().cast::<Work>() };
        let mut s = SketchW { w };
        s.clear();
        Some(s)
    }

    /// No rows (a new sketch's state).
    pub fn clear(&mut self) {
        let w = &mut *self.w;
        w.rows = 0;
        w.form = FORM_FEW;
        w.few_len = 0;
        w.registers.fill(0);
        w.counters.fill(0);
    }

    fn count_n(&mut self, h: u64, n: i64) {
        let (b, s) = bucket(h);
        self.w.counters[b] = self.w.counters[b].wrapping_add(s.wrapping_mul(n as i32));
    }

    fn register(&self, r: usize) -> u8 {
        self.w.ranks[r * RANKS..(r + 1) * RANKS].iter().rposition(|&n| n > 0).map_or(0, |i| i as u8 + 1)
    }

    /// Rank counts begun: all 0, filled by `rank_put` from its exact counts, then `ranks_made`.
    fn ranks_begin(&mut self) {
        self.w.ranks.fill(0);
    }

    fn rank_put(&mut self, x: i64, n: u64) {
        let (r, rho) = place(crate::bloom::hash(x));
        self.w.ranks[cell(r, rho)] += n as u32;
    }

    /// Its registers from its rank counts: its exact counts ended.
    fn ranks_made(&mut self) {
        for r in 0..REGISTERS {
            self.w.registers[r] = self.register(r);
        }
        self.w.form = FORM_RANKS;
        self.w.few_len = PAST;
    }

    // -- the few values

    fn few_add(&mut self, v: i64, n: i64) -> bool {
        let w = &mut *self.w;
        let len = w.few_len as usize;
        if let Some(e) = w.few[..len].iter_mut().find(|e| e[0] == v) {
            e[1] += n;
            return true;
        }
        if len == FEW {
            return false;
        }
        w.few[len] = [v, n];
        w.few_len += 1;
        true
    }

    fn few_remove(&mut self, v: i64, n: i64) -> Option<i64> {
        let w = &mut *self.w;
        let e = w.few[..w.few_len as usize].iter_mut().find(|e| e[0] == v)?;
        if e[1] <= 0 {
            return None;
        }
        e[1] = (e[1] - n).max(0);
        Some(e[1])
    }

    // -- a page

    fn page_at(&self, v: i64) -> Option<usize> {
        v.checked_sub(self.w.page_base).and_then(|n| usize::try_from(n).ok()).filter(|&i| i < DENSE_WIDTH)
    }

    fn page_add(&mut self, v: i64, k: u64) -> bool {
        let Some(i) = self.page_at(v) else { return false };
        let w = &mut *self.w;
        let n = u64::from(w.page[i]);
        if n == 0 {
            w.page_ndv += 1;
        }
        w.page_sq += 2 * n * k + k * k;
        w.page[i] += k as u32;
        true
    }

    fn page_remove(&mut self, v: i64, k: u64) {
        let Some(i) = self.page_at(v) else { return };
        let w = &mut *self.w;
        let n = u64::from(w.page[i]);
        let k = k.min(n);
        if k == 0 {
            return;
        }
        w.page_sq -= 2 * n * k - k * k;
        w.page[i] -= k as u32;
        if w.page[i] == 0 {
            w.page_ndv -= 1;
        }
    }

    // -- a label set

    fn label_slot(&self, v: i64) -> usize {
        let m = self.w.label_cap as usize - 1;
        let mut at = (crate::bloom::hash(v) as usize) & m;
        while self.w.labels[at][0] != NO_VALUE && self.w.labels[at][0] != v {
            at = (at + 1) & m;
        }
        at
    }

    /// Room for one more value: its slots doubled, each entry placed again in slot order (its old slots held in the
    /// rank counts' words meanwhile, four a slot: that part is no part of this form).
    fn label_grow(&mut self) {
        let cap = self.w.label_cap as usize;
        if 2 * (self.w.label_len as usize + 1) <= cap || cap >= SPARSE_SLOTS {
            return;
        }
        let w = &mut *self.w;
        for (i, e) in w.labels[..cap].iter().enumerate() {
            for (j, x) in [e[0] as u64, e[1] as u64].into_iter().enumerate() {
                w.ranks[4 * i + 2 * j] = x as u32;
                w.ranks[4 * i + 2 * j + 1] = (x >> 32) as u32;
            }
        }
        w.label_cap = 2 * cap as u64;
        w.labels[..2 * cap].fill([NO_VALUE, 0]);
        w.label_len = 0;
        for i in 0..cap {
            let word = |j: usize| (u64::from(self.w.ranks[4 * i + 2 * j]) | (u64::from(self.w.ranks[4 * i + 2 * j + 1]) << 32)) as i64;
            let e = [word(0), word(1)];
            if e[0] != NO_VALUE {
                let at = self.label_slot(e[0]);
                self.w.labels[at] = e;
                self.w.label_len += 1;
            }
        }
    }

    /// A label set begun empty, filled by `label_put` as `Sparse::from_counts` fills one.
    fn labels_begin(&mut self) {
        let w = &mut *self.w;
        w.label_cap = SPARSE_FIRST as u64;
        w.labels[..SPARSE_FIRST].fill([NO_VALUE, 0]);
        (w.label_len, w.label_sq, w.label_zero) = (0, 0, 0);
        w.form = FORM_LABELS;
        w.few_len = PAST;
    }

    fn label_put(&mut self, v: i64, n: u64) {
        if n == 0 {
            return;
        }
        self.label_grow();
        let at = self.label_slot(v);
        if self.w.labels[at][0] == NO_VALUE {
            self.w.label_len += 1;
        }
        self.w.labels[at] = [v, n as i64];
        self.w.label_sq += n * n;
    }

    fn label_add(&mut self, v: i64, k: u64) -> bool {
        if !labelled(v) {
            return false;
        }
        if self.w.label_len as usize >= SPARSE_LIMIT && self.w.labels[self.label_slot(v)][0] == NO_VALUE {
            return false;
        }
        self.label_grow();
        let at = self.label_slot(v);
        let w = &mut *self.w;
        if w.labels[at][0] == NO_VALUE {
            w.labels[at] = [v, 0];
            w.label_len += 1;
        } else if w.labels[at][1] == 0 {
            w.label_zero -= 1;
        }
        let n = w.labels[at][1] as u64;
        w.label_sq += 2 * n * k + k * k;
        w.labels[at][1] += k as i64;
        true
    }

    fn label_remove(&mut self, v: i64, k: u64) {
        if !labelled(v) {
            return;
        }
        let at = self.label_slot(v);
        let w = &mut *self.w;
        let n = w.labels[at][1] as u64;
        let k = k.min(n);
        if w.labels[at][0] != v || k == 0 {
            return;
        }
        w.label_sq -= 2 * n * k - k * k;
        w.labels[at][1] -= k as i64;
        if n == k {
            w.label_zero += 1;
        }
    }

    // -- adds and removes, as `Sketch::add_n` and `Sketch::remove_n`

    /// A row of `v`.
    pub fn add(&mut self, v: i64) {
        self.add_n(v, 1);
    }

    /// `n` rows of `v`.
    pub fn add_n(&mut self, v: i64, n: u64) {
        if n == 0 {
            return;
        }
        let h = crate::bloom::hash(v);
        let (r, rho) = place(h);
        if rho > self.w.registers[r] {
            self.w.registers[r] = rho;
        }
        if self.w.form == FORM_RANKS {
            self.w.ranks[cell(r, rho)] += n as u32;
        }
        self.count_n(h, n as i64);
        self.w.rows += n;
        match self.w.form {
            FORM_LABELS => {
                if !self.label_add(v, n) {
                    self.ranks_begin();
                    for i in 0..self.w.label_cap as usize {
                        let e = self.w.labels[i];
                        if e[0] != NO_VALUE && e[1] > 0 {
                            self.rank_put(e[0], e[1] as u64);
                        }
                    }
                    self.rank_put(v, n);
                    self.ranks_made();
                }
            }
            FORM_PAGE => {
                if !self.page_add(v, n) {
                    let labels = labelled(v) && (self.w.page_ndv as usize) < SPARSE_LIMIT;
                    if labels {
                        self.labels_begin();
                    } else {
                        self.ranks_begin();
                    }
                    for i in 0..DENSE_WIDTH {
                        let k = u64::from(self.w.page[i]);
                        if k > 0 {
                            let x = self.w.page_base + i as i64;
                            if labels { self.label_put(x, k) } else { self.rank_put(x, k) }
                        }
                    }
                    if labels {
                        self.label_add(v, n);
                    } else {
                        self.rank_put(v, n);
                        self.ranks_made();
                    }
                }
            }
            // The add is a side effect of its test: kept out of a match guard.
            #[allow(clippy::collapsible_match)]
            FORM_FEW => {
                if !self.few_add(v, n as i64) {
                    self.end_few(v, n);
                }
            }
            _ => {}
        }
    }

    /// Its `FEW + 1`th value: a page when they lie in one, else a label set when they are labels, else rank counts.
    fn end_few(&mut self, v: i64, n: u64) {
        let base = v & !((DENSE_WIDTH as i64) - 1);
        let len = self.w.few_len as usize;
        let small = &self.w.few[..len];
        if labelled(v) && small.iter().all(|e| e[0] >= base && e[0] - base < DENSE_WIDTH as i64) {
            let w = &mut *self.w;
            w.page.fill(0);
            w.page_base = base;
            w.page_ndv = w.few[..len].iter().filter(|e| e[1] > 0).count() as u64;
            w.page_sq = 0;
            for i in 0..len {
                let [x, k] = w.few[i];
                w.page[(x - base) as usize] = k as u32;
                w.page_sq += (k as u64).pow(2);
            }
            self.page_add(v, n);
            self.w.form = FORM_PAGE;
            self.w.few_len = PAST;
        } else if labelled(v) && small.iter().all(|e| labelled(e[0])) {
            // The few values read before the label set's begin marks them ended.
            self.labels_begin();
            for i in 0..len {
                let e = self.w.few[i];
                self.label_put(e[0], e[1] as u64);
            }
            self.label_add(v, n);
        } else {
            self.ranks_begin();
            for i in 0..len {
                let e = self.w.few[i];
                self.rank_put(e[0], e[1] as u64);
            }
            self.rank_put(v, n);
            self.ranks_made();
        }
    }

    /// A row of `v` taken out.
    pub fn remove(&mut self, v: i64) {
        self.remove_n(v, 1);
    }

    /// `n` rows of `v` taken out, as they were put in.
    pub fn remove_n(&mut self, v: i64, n: u64) {
        let n = n.min(self.w.rows);
        if n == 0 {
            return;
        }
        let h = crate::bloom::hash(v);
        let (r, rho) = place(h);
        self.count_n(h, -(n as i64));
        self.w.rows -= n;
        match self.w.form {
            FORM_LABELS => self.label_remove(v, n),
            FORM_PAGE => self.page_remove(v, n),
            FORM_FEW => {
                if self.few_remove(v, n as i64) == Some(0) {
                    // A value with no rows left: the few values' registers made again from those with rows.
                    let w = &mut *self.w;
                    w.registers.fill(0);
                    for e in w.few[..w.few_len as usize].iter().filter(|e| e[1] > 0) {
                        let (r, rho) = place(crate::bloom::hash(e[0]));
                        w.registers[r] = w.registers[r].max(rho);
                    }
                }
            }
            _ => {
                let at = cell(r, rho);
                self.w.ranks[at] = self.w.ranks[at].saturating_sub(n as u32);
                if self.w.ranks[at] == 0 && usize::from(rho).min(RANKS) >= usize::from(self.w.registers[r]) {
                    self.w.registers[r] = self.register(r);
                }
            }
        }
    }

    // -- its kept bytes

    /// The sketch whose kept bytes are `bytes` (`Sketch::decode`'s): false when they are not a sketch's of this
    /// format (it is then cleared).
    pub fn decode(&mut self, bytes: &[u8]) -> bool {
        let ok = self.decode_in(bytes).is_some();
        if !ok {
            self.clear();
        }
        ok
    }

    fn decode_in(&mut self, bytes: &[u8]) -> Option<()> {
        let at = core::cell::Cell::new(0usize);
        let mut next = || -> Option<u8> {
            let b = *bytes.get(at.get())?;
            at.set(at.get() + 1);
            Some(b)
        };
        if next()? != SKETCH_FORMAT {
            return None;
        }
        let form = next()?;
        self.clear();
        self.w.rows = read_varint(&mut next)?;
        for r in 0..REGISTERS {
            self.w.registers[r] = next()?;
        }
        for b in 0..BUCKETS {
            self.w.counters[b] = unzigzag(read_varint(&mut next)?) as i32;
        }
        let count = read_varint(&mut next)? as usize;
        match u64::from(form) {
            FORM_FEW => {
                if count > FEW {
                    return None;
                }
                for k in 0..count {
                    let v = unzigzag(read_varint(&mut next)?);
                    self.w.few[k] = [v, read_varint(&mut next)? as i64];
                    self.w.few_len = k as u64 + 1;
                }
            }
            FORM_PAGE => {
                let w = &mut *self.w;
                w.page_base = unzigzag(read_varint(&mut next)?);
                w.page.fill(0);
                (w.page_ndv, w.page_sq) = (0, 0);
                let mut i = 0usize;
                for e in 0..count {
                    let step = read_varint(&mut next)? as usize;
                    i += step;
                    if e > 0 && step == 0 || i >= DENSE_WIDTH {
                        return None;
                    }
                    let k = read_varint(&mut next)?;
                    w.page[i] = k as u32;
                    w.page_ndv += 1;
                    w.page_sq += k * k;
                }
                w.form = FORM_PAGE;
                w.few_len = PAST;
            }
            FORM_LABELS => {
                // Read once to check them (as the decoder gathers them, then checks), then made as it makes them.
                let from = at.get();
                for _ in 0..count {
                    let v = read_varint(&mut next)? as i64;
                    read_varint(&mut next)?;
                    if !labelled(v) {
                        return None;
                    }
                }
                if count > SPARSE_LIMIT {
                    return None;
                }
                let entries = &bytes[from..at.get()];
                let mut i = 0usize;
                let mut more = || -> Option<u8> {
                    let b = *entries.get(i)?;
                    i += 1;
                    Some(b)
                };
                self.labels_begin();
                for _ in 0..count {
                    let v = read_varint(&mut more)? as i64;
                    let k = read_varint(&mut more)?;
                    self.label_put(v, k);
                }
            }
            FORM_RANKS => {
                let w = &mut *self.w;
                w.ranks.fill(0);
                let mut i = 0usize;
                for e in 0..count {
                    let step = read_varint(&mut next)? as usize;
                    i += step;
                    if e > 0 && step == 0 {
                        return None;
                    }
                    *w.ranks.get_mut(i)? = read_varint(&mut next)? as u32;
                }
                w.form = FORM_RANKS;
                w.few_len = PAST;
            }
            _ => return None,
        }
        (at.get() == bytes.len()).then_some(())
    }

    /// Its kept bytes (`Sketch::encode`'s), each given to `out` in turn.
    fn emit(&self, out: &mut dyn FnMut(u8)) {
        let w = &*self.w;
        let varint = |out: &mut dyn FnMut(u8), mut v: u64| {
            while v >= 0x80 {
                out((v as u8) | 0x80);
                v >>= 7;
            }
            out(v as u8);
        };
        out(SKETCH_FORMAT);
        out(w.form as u8);
        varint(out, w.rows);
        for &r in &w.registers {
            out(r);
        }
        for &c in &w.counters {
            varint(out, zigzag(i64::from(c)));
        }
        match w.form {
            FORM_FEW => {
                varint(out, w.few_len);
                for &[v, n] in &w.few[..w.few_len as usize] {
                    varint(out, zigzag(v));
                    varint(out, n as u64);
                }
            }
            FORM_PAGE => {
                varint(out, w.page.iter().filter(|&&n| n > 0).count() as u64);
                varint(out, zigzag(w.page_base));
                let mut last = 0;
                for (i, &n) in w.page.iter().enumerate().filter(|e| *e.1 > 0) {
                    varint(out, (i - last) as u64);
                    varint(out, u64::from(n));
                    last = i;
                }
            }
            FORM_LABELS => {
                let live = |e: &&[i64; 2]| e[0] != NO_VALUE && e[1] > 0;
                let slots = &w.labels[..w.label_cap as usize];
                varint(out, slots.iter().filter(live).count() as u64);
                for e in slots.iter().filter(live) {
                    varint(out, e[0] as u64);
                    varint(out, e[1] as u64);
                }
            }
            _ => {
                varint(out, w.ranks.iter().filter(|&&n| n > 0).count() as u64);
                let mut last = 0;
                for (i, &n) in w.ranks.iter().enumerate().filter(|e| *e.1 > 0) {
                    varint(out, (i - last) as u64);
                    varint(out, u64::from(n));
                    last = i;
                }
            }
        }
    }

    /// Its kept bytes' length.
    pub fn encoded_len(&self) -> usize {
        let mut n = 0;
        self.emit(&mut |_| n += 1);
        n
    }

    /// Its kept bytes, written into `to` (exactly `encoded_len` long).
    pub fn encode_into(&self, to: &mut [u8]) {
        let mut at = 0;
        self.emit(&mut |b| {
            to[at] = b;
            at += 1;
        });
        assert_eq!(at, to.len(), "a sketch's kept bytes written whole");
    }
}
