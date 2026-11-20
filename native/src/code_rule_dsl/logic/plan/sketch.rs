//! Deletable linear sketches of a column's values, for the planner's statistics (decision 0096).
//!
//! The planner reads, per column, the distinct values (`ndv`) and the sum of squared postings (`sq`, whose ratio to
//! the rows is an equality probe's fanout). Kept exactly, both need a count per distinct value. These sketches keep
//! them in a fixed size instead, within a small factor:
//!
//! ```text
//! sq:  AMS / Count-Sketch: D rows of W signed counters. A value adds ±n (sign from its hash) at one counter per row.
//!      sq ≈ median over rows of Σ counter².     // a heavy value lands whole in one counter: never sampled away
//! ndv: a levelled counting bitmap. level(v) = trailing zeros of hash(v), so levels ≥ L hold a 2^-L sample of the
//!      distinct values; each level has M counters, and v adds n at one of them (counts, not bits: deletes work).
//!      ndv ≈ 2^L · −M·ln(empty/M), at the lowest L whose union leaves enough slots empty.
//! ```
//!
//! Both are linear: a row's insert adds to counters and its delete subtracts the same. The counters after any order
//! of inserts and deletes are those of the final multiset (history-independent), two sketches add to the sketch of
//! both, and a document's rows are taken out by subtracting them.

/// Counters per row of the AMS sketch.
pub(in crate::code_rule_dsl) const W: usize = 64;
/// Rows of the AMS sketch (the estimate is their median).
pub(in crate::code_rule_dsl) const D: usize = 3;
/// Counters per level of the distinct-value bitmap.
pub(in crate::code_rule_dsl) const M: usize = 64;
const LEVELS: usize = 32;
/// The stored format (`encode`): 2 places a value by one mix (`add`).
const FORMAT: u8 = 2;
const _: () = assert!(W == 64 && M == 64 && D == 3, "`add` takes 6 bits a counter and 7 an AMS row, of three");

/// One column's sketches, in a block (decision 0478 R5, Q338): `words` holds the AMS counters (`D` rows of `W`), then
/// a page of `M` counters for each level a value reached, in the order they were first reached. `page[level]` is that
/// level's page (one past it; 0 for none), and `open` the levels holding a count now: a level whose counters all
/// went back to zero keeps its page, zeroed, but is not open.
#[derive(Clone)]
pub struct Sketch {
    words: crate::code_rule_dsl::facts::rows::BlockVec<i64>,
    page: [u8; LEVELS],
    open: u32,
}

impl Default for Sketch {
    fn default() -> Self {
        let mut words = crate::code_rule_dsl::facts::rows::BlockVec::new();
        words.resize(D * W, 0);
        Sketch { words, page: [0; LEVELS], open: 0 }
    }
}

/// Two sketches are equal when their counters are: the same AMS counters and the same open levels' counts.
impl PartialEq for Sketch {
    fn eq(&self, other: &Self) -> bool {
        self.words[..D * W] == other.words[..D * W]
            && self.open == other.open
            && (0..LEVELS).filter(|l| self.open & 1 << l != 0).all(|l| self.level(l) == other.level(l))
    }
}
impl Eq for Sketch {}
impl std::fmt::Debug for Sketch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sketch").field("ams", &&self.words[..D * W]).field("open", &format_args!("{:#x}", self.open)).finish()
    }
}

/// `mix` of a value: the slot an exact count's value hashes to (`statkeep::Exact`).
pub fn mix_of(v: i64) -> u64 {
    mix(v as u64)
}

/// splitmix64: a fixed mixer, so a sketch is the same in every process.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Sketch {
    /// Level `level`'s counters, when it holds a count.
    fn level(&self, level: usize) -> Option<&[i64]> {
        (self.open & 1 << level != 0).then(|| {
            let at = D * W + (usize::from(self.page[level]) - 1) * M;
            &self.words[at..at + M]
        })
    }
    /// Level `level`'s counters to add to, its page made if it has none, and the level open.
    fn level_mut(&mut self, level: usize) -> &mut [i64] {
        if self.page[level] == 0 {
            let pages = (self.words.len() - D * W) / M;
            self.words.resize(self.words.len() + M, 0);
            self.page[level] = (pages + 1) as u8;
        }
        self.open |= 1 << level;
        let at = D * W + (usize::from(self.page[level]) - 1) * M;
        &mut self.words[at..at + M]
    }
    /// Level `level` closed if its counters are all zero again.
    fn settle(&mut self, level: usize) {
        if self.level(level).is_some_and(|c| c.iter().all(|&x| x == 0)) {
            self.open &= !(1 << level);
        }
    }

    /// `n` rows of value `v` in (`n < 0`: taken out).
    pub fn add(&mut self, v: i64, n: i64) {
        // One mix a value: each AMS row's sign and counter from 7 bits of it (bits 0–20), the bitmap's slot from 6
        // more (21–26), and its level from the rest's trailing zeros (format 2; format 1 mixed once a row and once
        // more for the bitmap, 7 mixes a value).
        let h = mix(v as u64);
        ams_add(&mut self.words[..D * W], h, n);
        let level = ((h >> 27).trailing_zeros() as usize).min(LEVELS - 1);
        let slot = ((h >> 21) % M as u64) as usize;
        self.level_mut(level)[slot] += n;
        if n < 0 {
            self.settle(level);
        }
    }

    /// `other`'s rows in too (`sign` −1: out).
    pub(in crate::code_rule_dsl) fn merge(&mut self, other: &Sketch, sign: i64) {
        for (x, y) in self.words[..D * W].iter_mut().zip(&other.words[..D * W]) {
            *x += sign * y;
        }
        for level in 0..LEVELS {
            let Some(theirs) = other.level(level) else { continue };
            for (x, y) in self.level_mut(level).iter_mut().zip(theirs) {
                *x += sign * y;
            }
            self.settle(level);
        }
    }

    /// The estimated sum of squared postings.
    pub fn sq(&self) -> f64 {
        ams_sq(&self.words[..D * W])
    }

    /// The estimated distinct values.
    pub fn ndv(&self) -> f64 {
        let mut occupied = [false; M];
        // From the top level down: the union of levels ≥ L, and the lowest L where linear counting still holds.
        let mut best = None;
        for level in (0..LEVELS).rev() {
            if let Some(c) = self.level(level) {
                for (o, &x) in occupied.iter_mut().zip(c.iter()) {
                    *o |= x != 0;
                }
            }
            let empty = occupied.iter().filter(|o| !**o).count();
            if empty * 10 < M * 3 {
                break;
            }
            best = Some((level, empty));
        }
        match best {
            Some((level, empty)) => (1u64 << level) as f64 * -(M as f64) * (empty as f64 / M as f64).ln(),
            None => f64::INFINITY,
        }
    }

    /// Bytes this sketch takes stored (`encode`).
    pub fn bytes(&self) -> usize {
        5 + D * W * 4 + self.open.count_ones() as usize * M * 4
    }

    /// The sketch as stored (`stat_total.sketch`): a format byte, the AMS counters, a mask of the occupied levels,
    /// and each occupied level's counters; counters as little-endian i32 (a column's rows stay under 2^31).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes());
        out.push(FORMAT);
        for &c in &self.words[..D * W] {
            out.extend_from_slice(&(c as i32).to_le_bytes());
        }
        out.extend_from_slice(&self.open.to_le_bytes());
        for c in (0..LEVELS).filter_map(|l| self.level(l)).flatten() {
            out.extend_from_slice(&(*c as i32).to_le_bytes());
        }
        out
    }

    /// `encode`'s sketch back; `None` if the bytes are not one.
    pub fn decode(bytes: &[u8]) -> Option<Sketch> {
        let mut at = 1;
        let mut int = || {
            let b = bytes.get(at..at + 4)?;
            at += 4;
            Some(i32::from_le_bytes(b.try_into().ok()?))
        };
        // A sketch of another format placed its values elsewhere: not one to add rows to.
        if bytes.first() != Some(&FORMAT) {
            return None;
        }
        let mut s = Sketch::default();
        for c in s.words[..D * W].iter_mut() {
            *c = i64::from(int()?);
        }
        let mask = int()? as u32;
        for level in 0..LEVELS {
            if mask & 1 << level != 0 {
                for c in s.level_mut(level).iter_mut() {
                    *c = i64::from(int()?);
                }
            }
        }
        (at == bytes.len()).then_some(s)
    }
}

/// HyperLogLog's registers (Flajolet, Fusy, Gandouet, Meunier 2007): 2^12, so a standard error of 1.6%.
const HLL_P: u32 = 12;

/// A column's moments where values are only ever added (the corpus's chunks' totals, which merge parts that share
/// values): `sq` from the AMS sketch above, and the distinct values by HyperLogLog, which cannot take a value out
/// but is far closer than the counting bitmap a deletable `Sketch` needs (1.6% against up to 1.7×).
#[derive(Clone)]
pub struct Moments {
    ams: [[i64; W]; D],
    regs: Box<[u8; 1 << HLL_P]>,
}

impl Default for Moments {
    fn default() -> Self {
        Moments { ams: [[0; W]; D], regs: Box::new([0; 1 << HLL_P]) }
    }
}

impl Moments {
    /// `n` rows of value `v` in (`n > 0`).
    pub fn add(&mut self, v: i64, n: i64) {
        let h = mix(v as u64);
        ams_add(self.ams.as_flattened_mut(), h, n);
        // Another mix for the registers, so they share no bits with the AMS sketch's choices.
        let g = mix(h ^ 0x5851_f42d_4c95_7f2d);
        let at = (g >> (64 - HLL_P)) as usize;
        let rank = ((g << HLL_P) | 1 << (HLL_P - 1)).leading_zeros() as u8 + 1;
        self.regs[at] = self.regs[at].max(rank);
    }

    /// The estimated sum of squared postings.
    pub fn sq(&self) -> f64 {
        ams_sq(self.ams.as_flattened())
    }

    /// The estimated distinct values: HyperLogLog's, by linear counting while registers are empty and it is small.
    pub fn ndv(&self) -> f64 {
        let m = (1u32 << HLL_P) as f64;
        let alpha = 0.7213 / (1.0 + 1.079 / m);
        let sum: f64 = self.regs.iter().map(|&r| (-(r as f64)).exp2()).sum();
        let raw = alpha * m * m / sum;
        let zeros = self.regs.iter().filter(|&&r| r == 0).count();
        if raw <= 2.5 * m && zeros > 0 { m * (m / zeros as f64).ln() } else { raw }
    }
}

/// `n` rows of the value hashed to `h` into the AMS counters.
fn ams_add(ams: &mut [i64], h: u64, n: i64) {
    for (i, row) in ams.chunks_exact_mut(W).enumerate() {
        let at = (((h >> (i * 7)) >> 1) % W as u64) as usize;
        row[at] += if (h >> (i * 7)) & 1 == 1 { n } else { -n };
    }
}

/// The AMS counters' estimate of the sum of squared postings: the median row's.
fn ams_sq(ams: &[i64]) -> f64 {
    let mut rows: Vec<f64> = ams.chunks_exact(W).map(|r| r.iter().map(|&c| (c as f64) * (c as f64)).sum()).collect();
    rows.sort_by(f64::total_cmp);
    rows[D / 2]
}

/// `RERAIL_STATS_SHADOW=<file>` (a measurement aid, decision 0096): every column measured exactly is also sketched,
/// and one line per column is appended: where, the column, its rows, exact and estimated ndv and sq, and the bytes.
pub fn shadowing() -> bool {
    shadow_file().is_some()
}

/// The run's own (opened once a run, decision 0442).
fn shadow_file() -> Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>> {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| {
        let path = std::env::var_os("RERAIL_STATS_SHADOW")?;
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(|f| std::sync::Arc::new(std::sync::Mutex::new(f)))
    })
}

/// One shadowed column's line (`shadowing`).
pub fn shadow(at: &str, col: &str, rows: i64, ndv: i64, sq: i64, sketch: &Sketch) {
    use std::io::Write as _;
    if let Some(f) = shadow_file() {
        let _ = writeln!(f.lock().unwrap(), "{at}\t{col}\t{rows}\t{ndv}\t{:.0}\t{sq}\t{:.0}\t{}", sketch.ndv(), sketch.sq(), sketch.bytes());
    }
}

#[cfg(test)]
pub mod tests {
    use super::{Moments, Sketch};

    /// Decision 0478 R5 (Q338): a sketch's counters are one block array, a level's page made where a value first
    /// reaches it; a level counted out again is closed, so the sketch equals one that never held those rows.
    #[test]
    fn a_planner_sketchs_counters_are_one_block_array() {
        let mut s = Sketch::default();
        for v in 0..5000 {
            s.add(v, 2);
        }
        assert!(s.words.facts().is_some() && s.open != 0, "its counters in a block");
        let mut t = s.clone();
        for v in 5000..5100 {
            t.add(v, 1);
        }
        for v in 5000..5100 {
            t.add(v, -1);
        }
        assert_eq!(t, s, "counted out again, the same sketch");
        assert_eq!(Sketch::decode(&s.encode()), Some(s));
    }

    /// A column's moments over parts that share values (the corpus's chunks): distinct values counted once,
    /// whichever parts hold them, and the squared postings of each value's total, not of its parts' counts.
    #[test]
    fn moments_count_a_value_shared_by_parts_once() {
        let mut m = Moments::default();
        let mut sq = 0i64;
        for v in 0..100_000i64 {
            let n = 1 + v % 7;
            sq += n * n;
            // Each value's rows split over two parts.
            m.add(v * 7919, n / 2);
            m.add(v * 7919, n - n / 2);
        }
        let ndv = m.ndv();
        assert!((ndv / 100_000.0 - 1.0).abs() < 0.05, "ndv {ndv} for 100,000");
        assert!(m.sq() / sq as f64 > 1.0 / 1.5 && m.sq() / (sq as f64) < 1.5, "sq {} for {sq}", m.sq());
        let mut few = Moments::default();
        for v in 0..10 {
            few.add(v, 3);
        }
        assert_eq!(few.ndv().round(), 10.0, "few values are counted exactly (linear counting)");
    }

    /// A skewed column: value v holds about 10000 / (v + 1) rows (Zipf), over `n` distinct values.
    fn zipf(n: i64) -> Vec<(i64, i64)> {
        (0..n).map(|v| (v * 7919 + 13, (10_000 / (v + 1)).max(1))).collect()
    }

    #[test]
    fn a_sketch_is_that_of_its_final_rows_whatever_the_order_of_inserts_and_deletes() {
        let rows = zipf(5000);
        let mut once = Sketch::default();
        for &(v, n) in &rows {
            once.add(v, n);
        }
        // Insert all one row at a time, take a third out, put it back, and take out values never kept.
        let mut churned = Sketch::default();
        for &(v, n) in &rows {
            for _ in 0..n.min(3) {
                churned.add(v, 1);
            }
            churned.add(v, n - n.min(3));
        }
        for &(v, n) in rows.iter().step_by(3) {
            churned.add(v, -n);
        }
        churned.add(999_999_999, 5);
        for &(v, n) in rows.iter().step_by(3) {
            churned.add(v, n);
        }
        churned.add(999_999_999, -5);
        assert_eq!(once, churned);
        // Parts add: two halves merged are the whole; a half subtracted leaves the other.
        let (mut a, mut b) = (Sketch::default(), Sketch::default());
        for (i, &(v, n)) in rows.iter().enumerate() {
            if i % 2 == 0 { a.add(v, n) } else { b.add(v, n) }
        }
        let mut both = a.clone();
        both.merge(&b, 1);
        assert_eq!(both, once);
        both.merge(&a, -1);
        assert_eq!(both, b);
        // Stored and read back, the same.
        assert_eq!(Sketch::decode(&once.encode()).as_ref(), Some(&once));
        assert_eq!(once.encode().len(), once.bytes());
    }

    #[test]
    fn a_sketch_estimates_skewed_and_uniform_columns_within_a_small_factor() {
        for (name, rows) in [("zipf", zipf(20_000)), ("uniform", (0..50_000).map(|v| (v, 3)).collect()), ("few", (0..40).map(|v| (v, 100 + v)).collect())] {
            let mut s = Sketch::default();
            for &(v, n) in &rows {
                s.add(v, n);
            }
            let ndv = rows.len() as f64;
            let sq: f64 = rows.iter().map(|&(_, n)| (n * n) as f64).sum();
            let (rn, rs) = (s.ndv() / ndv, s.sq() / sq);
            assert!((0.6..1.6).contains(&rn), "{name}: ndv estimate {} of {ndv}", s.ndv());
            assert!((0.6..1.6).contains(&rs), "{name}: sq estimate {} of {sq}", s.sq());
        }
    }
}
