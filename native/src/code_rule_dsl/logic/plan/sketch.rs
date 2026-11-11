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

/// One column's sketches.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sketch {
    ams: [[i64; W]; D],
    /// Per level, M counters; a level is allocated when a value first reaches it (most levels stay empty).
    levels: Vec<Option<Box<[i64; M]>>>,
}

impl Default for Sketch {
    fn default() -> Self {
        Sketch { ams: [[0; W]; D], levels: vec![None; LEVELS] }
    }
}

/// splitmix64: a fixed mixer, so a sketch is the same in every process.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Sketch {
    /// `n` rows of value `v` in (`n < 0`: taken out).
    pub fn add(&mut self, v: i64, n: i64) {
        for (i, row) in self.ams.iter_mut().enumerate() {
            let h = mix((v as u64) ^ mix(i as u64 + 1));
            let sign = if h >> 63 == 1 { 1 } else { -1 };
            row[(h % W as u64) as usize] += sign * n;
        }
        let h = mix((v as u64) ^ 0x5bd1_e995);
        let level = (h.trailing_zeros() as usize).min(LEVELS - 1);
        let slot = ((h >> 32) % M as u64) as usize;
        let counters = self.levels[level].get_or_insert_with(|| Box::new([0; M]));
        counters[slot] += n;
        if n < 0 && counters.iter().all(|&c| c == 0) {
            self.levels[level] = None;
        }
    }

    /// `other`'s rows in too (`sign` −1: out).
    pub(in crate::code_rule_dsl) fn merge(&mut self, other: &Sketch, sign: i64) {
        for (a, b) in self.ams.iter_mut().zip(&other.ams) {
            for (x, y) in a.iter_mut().zip(b) {
                *x += sign * y;
            }
        }
        for (level, theirs) in other.levels.iter().enumerate() {
            let Some(theirs) = theirs else { continue };
            let ours = self.levels[level].get_or_insert_with(|| Box::new([0; M]));
            for (x, y) in ours.iter_mut().zip(theirs.iter()) {
                *x += sign * y;
            }
            if ours.iter().all(|&c| c == 0) {
                self.levels[level] = None;
            }
        }
    }

    /// The estimated sum of squared postings.
    pub fn sq(&self) -> f64 {
        let mut rows: Vec<f64> = self.ams.iter().map(|r| r.iter().map(|&c| (c as f64) * (c as f64)).sum()).collect();
        rows.sort_by(f64::total_cmp);
        rows[D / 2]
    }

    /// The estimated distinct values.
    pub fn ndv(&self) -> f64 {
        let mut occupied = [false; M];
        // From the top level down: the union of levels ≥ L, and the lowest L where linear counting still holds.
        let mut best = None;
        for level in (0..LEVELS).rev() {
            if let Some(c) = &self.levels[level] {
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
        5 + D * W * 4 + self.levels.iter().flatten().count() * M * 4
    }

    /// The sketch as stored (`stat_total.sketch`): a format byte, the AMS counters, a mask of the occupied levels,
    /// and each occupied level's counters; counters as little-endian i32 (a column's rows stay under 2^31).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes());
        out.push(1);
        for &c in self.ams.iter().flatten() {
            out.extend_from_slice(&(c as i32).to_le_bytes());
        }
        let mask = self.levels.iter().enumerate().filter(|(_, l)| l.is_some()).fold(0u32, |m, (i, _)| m | 1 << i);
        out.extend_from_slice(&mask.to_le_bytes());
        for &c in self.levels.iter().flatten().flat_map(|l| l.iter()) {
            out.extend_from_slice(&(c as i32).to_le_bytes());
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
        if bytes.first() != Some(&1) {
            return None;
        }
        let mut s = Sketch::default();
        for c in s.ams.iter_mut().flatten() {
            *c = i64::from(int()?);
        }
        let mask = int()? as u32;
        for level in 0..LEVELS {
            if mask & 1 << level != 0 {
                let mut l = Box::new([0; M]);
                for c in l.iter_mut() {
                    *c = i64::from(int()?);
                }
                s.levels[level] = Some(l);
            }
        }
        (at == bytes.len()).then_some(s)
    }
}

/// `RERAIL_STATS_SHADOW=<file>` (a measurement aid, decision 0096): every column measured exactly is also sketched,
/// and one line per column is appended: where, the column, its rows, exact and estimated ndv and sq, and the bytes.
pub fn shadowing() -> bool {
    shadow_file().is_some()
}

fn shadow_file() -> Option<&'static std::sync::Mutex<std::fs::File>> {
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::var_os("RERAIL_STATS_SHADOW")?;
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(std::sync::Mutex::new)
    })
    .as_ref()
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
    use super::Sketch;

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
