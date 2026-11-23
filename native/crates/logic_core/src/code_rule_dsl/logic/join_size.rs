//! A join's size from its columns' count sketches (decision 0488): the AMS estimate (Alon, Matias and Szegedy; Alon,
//! Gibbons, Matias and Szegedy's "tug of war"), from counters every column's sketch keeps already
//! (`native::setfix::sketch::Sketch`), as its rows come, at no cost a probe pays.
//!
//! ```text
//! a key's bucket and sign:  b = (h >> 8) & 255,  s = +1 if h & 1 == 0 else -1     (h = bloom::hash(key), the one hash)
//! a column's counters:      C[b] = sum of s over its rows of bucket b
//! the join's rows:          J = sum over keys v of a_v b_v  ~  sum over b of A[b] B[b]       (unbiased)
//! its error:                Var = (F2(A) F2(B) + J^2 - 2 sum a_v^2 b_v^2) / 256  <=  (F2(A) F2(B) + J^2) / 256
//! ```
//!
//! A column of at most `EXACT` values keeps their counts too: two such give J exactly, and one gives each of its
//! values' rows on the other side by the other's counters (a point query, of the same error bound).
//!
//! The planner prices a probe by one atom's column at `J / rows` of that atom a binding: rows that meet and bindings
//! that miss alike (`plan::Model::atom_rows`), where the error is a fraction of J (`Joined::tells`).

use rerail_blocks::vector::Vector;
/// The counters a column keeps.
pub const BUCKETS: usize = 256;
/// The values a column keeps exact counts of, at most.
pub const EXACT: usize = 64;

/// A key's counter and its sign there, from its hash (`bloom::hash`).
#[inline(always)]
pub fn bucket(h: u64) -> (usize, i32) {
    (((h >> 8) & 0xff) as usize, if h & 1 == 0 { 1 } else { -1 })
}

/// A column's join-size counts, in a block (decision 0478 step 6c): its counters (`BUCKETS`) and, while it has at
/// most `EXACT` values, each value's rows (`[value, rows]`). Its sketch (`native::setfix::sketch::Sketch`) is its one
/// writer, as the rows come; the planner's `JoinCounts` and the statistics' keys (`bloom::Keys::Few`) read it where
/// it lies, held by both (decision 0353: read in place, never copied). Every cell is read and written as an atomic of
/// its width, relaxed: the writer's stores are plain stores, and a reader on another thread reads a value whole.
pub struct Counts {
    counters: rerail_blocks::vector::Vector<i32>,
    few: rerail_blocks::vector::Vector<[i64; 2]>,
    /// `few`'s values in use, or `PAST` once the column has had more than `EXACT`.
    len: std::sync::atomic::AtomicU32,
}

/// `Counts::len` once its values outgrew `EXACT`: the counters alone tell its joins.
const PAST: u32 = u32::MAX;

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicI32, AtomicI64};

impl Counts {
    /// No rows: its arrays at their full size, in a block, so they never move while they are read.
    pub fn new() -> Counts {
        let mut counters = crate::code_rule_dsl::facts::rows::vector_in(None, 0);
        counters.resize(BUCKETS, 0i32);
        let mut few = crate::code_rule_dsl::facts::rows::vector_in(None, 0);
        few.resize(EXACT, [0i64; 2]);
        Counts { counters, few, len: std::sync::atomic::AtomicU32::new(0) }
    }

    fn counter(&self, b: usize) -> &AtomicI32 {
        assert!(b < BUCKETS);
        // SAFETY: counter `b` of the array, which lives and stays where it is while `self` does; an `i32`'s layout
        // is an `AtomicI32`'s, and every access to it is atomic.
        unsafe { &*self.counters.as_ptr().cast::<AtomicI32>().add(b) }
    }

    fn few_cell(&self, i: usize) -> &[AtomicI64; 2] {
        assert!(i < EXACT);
        // SAFETY: as `counter`, for entry `i` of `few`.
        unsafe { &*self.few.as_ptr().cast::<[AtomicI64; 2]>().add(i) }
    }

    /// Adds a row of the key whose hash is `h` to its counter. Its one writer only.
    #[inline(always)]
    pub fn count(&self, h: u64) {
        let (b, s) = bucket(h);
        let c = self.counter(b);
        c.store(c.load(Relaxed) + s, Relaxed);
    }

    /// Adds a row of value `v` to the exact counts, while they are kept: false when `v` would be value `EXACT + 1`
    /// (nothing changed: the writer then ends them, `end_few`, once it has read them). Its one writer only.
    pub fn add_few(&self, v: i64) -> bool {
        let n = self.len.load(Relaxed);
        if n == PAST {
            return false;
        }
        for i in 0..n as usize {
            let e = self.few_cell(i);
            if e[0].load(Relaxed) == v {
                e[1].store(e[1].load(Relaxed) + 1, Relaxed);
                return true;
            }
        }
        if n as usize == EXACT {
            return false;
        }
        let e = self.few_cell(n as usize);
        e[0].store(v, Relaxed);
        e[1].store(1, Relaxed);
        // Released: a reader that sees the length sees the entry.
        self.len.store(n + 1, std::sync::atomic::Ordering::Release);
        true
    }

    /// Ends the exact counts: past `EXACT` values, the counters alone tell its joins. Its one writer only.
    pub fn end_few(&self) {
        self.len.store(PAST, std::sync::atomic::Ordering::Release);
    }

    /// Each value's rows (`[value, rows]`), while it keeps them, read where they lie.
    pub fn few(&self) -> Option<impl Iterator<Item = [i64; 2]> + '_> {
        let n = self.len.load(std::sync::atomic::Ordering::Acquire);
        (n != PAST).then(|| (0..n as usize).map(|i| self.few_cell(i)).map(|e| [e[0].load(Relaxed), e[1].load(Relaxed)]))
    }

    /// Its values while it keeps them: how many.
    pub fn few_len(&self) -> Option<usize> {
        let n = self.len.load(std::sync::atomic::Ordering::Acquire);
        (n != PAST).then_some(n as usize)
    }

    /// Its counters, read where they lie.
    pub fn counters(&self) -> impl Iterator<Item = i32> + '_ {
        (0..BUCKETS).map(|b| self.counter(b).load(Relaxed))
    }

    /// Its arrays' bytes.
    pub fn bytes(&self) -> usize {
        4 * BUCKETS + 16 * EXACT
    }

    /// Whether its arrays are in a block (decision 0478 step 6c).
    pub fn in_blocks(&self) -> bool {
        use rerail_blocks::maps::Placed;
        self.counters.place().is_some() && self.few.place().is_some()
    }
}

/// A new writer's own counts, from these (a sketch's clone at its first add: its filter is made so too, `Kept`'s
/// `Clone`): written once, by that writer, in a block of its own.
impl Clone for Counts {
    fn clone(&self) -> Counts {
        let made = Counts::new();
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

/// A column's counts as the planner reads them: its sketch's `Counts`, read where they lie, with its rows and sum
/// of squared rows a value (`F2`) as the statistics read them.
#[derive(Clone)]
pub struct JoinCounts {
    counts: std::sync::Arc<Counts>,
    pub rows: f64,
    pub f2: f64,
}

impl std::fmt::Debug for JoinCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JoinCounts {{ rows: {}, f2: {}, few: {:?} }}", self.rows, self.f2, self.counts.few_len())
    }
}

/// A join's rows, estimated, and the estimate's standard error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joined {
    pub rows: f64,
    pub err: f64,
}

impl Joined {
    /// Whether it tells the join's size: exact, or its error under half of it.
    pub fn tells(&self) -> bool {
        self.err == 0.0 || self.err < 0.5 * self.rows
    }
}

impl JoinCounts {
    /// A sketch's counts, held (not copied), with its rows and its `F2`.
    pub fn new(counts: std::sync::Arc<Counts>, rows: u64, f2: u64) -> JoinCounts {
        JoinCounts { counts, rows: rows as f64, f2: f2 as f64 }
    }

    /// A column's counts, from its values (as a sketch's adds make them: for tests and columns the engine does not
    /// sketch).
    pub fn of(values: impl IntoIterator<Item = i64>) -> JoinCounts {
        let counts = Counts::new();
        let mut each: std::collections::HashMap<i64, u64> = std::collections::HashMap::new();
        for v in values {
            counts.count(super::bloom::hash(v));
            if !counts.add_few(v) {
                counts.end_few();
            }
            *each.entry(v).or_default() += 1;
        }
        let rows = each.values().sum();
        let f2 = each.values().map(|&n| n * n).sum();
        JoinCounts::new(std::sync::Arc::new(counts), rows, f2)
    }

    /// Value `v`'s rows: exact where its counts are kept, else its counter's, signed (unbiased; others of its
    /// bucket add their noise).
    fn point(&self, v: i64) -> f64 {
        match self.counts.few() {
            Some(mut few) => few.find(|e| e[0] == v).map_or(0.0, |e| e[1] as f64),
            None => {
                let (b, s) = bucket(super::bloom::hash(v));
                f64::from(self.counts.counter(b).load(Relaxed) * s)
            }
        }
    }

    /// The rows of the equi-join of this column with `other`: exact where both keep exact counts; else the AMS
    /// estimate (one side's values point-queried on the other's counters, where one keeps them), within the bounds
    /// any join of the two has (none, and `sqrt(F2 F2)` by Cauchy-Schwarz).
    pub fn join(&self, other: &JoinCounts) -> Joined {
        let bound = (self.f2 * other.f2).sqrt();
        let exact = |c: &JoinCounts, o: &JoinCounts| -> Option<f64> { Some(c.counts.few()?.map(|[v, n]| n as f64 * o.point(v)).sum()) };
        let (rows, err) = if self.counts.few_len().is_some() && other.counts.few_len().is_some() {
            (exact(self, other).unwrap_or(0.0), 0.0)
        } else if let Some(j) = exact(self, other).or_else(|| exact(other, self)) {
            (j, (bound * bound / BUCKETS as f64).sqrt())
        } else {
            let j: f64 = self.counts.counters().zip(other.counts.counters()).map(|(a, b)| f64::from(a) * f64::from(b)).sum();
            (j, ((bound * bound + j * j) / BUCKETS as f64).sqrt())
        };
        Joined { rows: rows.clamp(0.0, bound), err }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two skewed columns of 20,000 rows, a few keys heavy in both: the estimate is the join's rows within its error,
    /// and its error a small part of them; two of few values join exactly; one of few values against the other's
    /// counters, within its error.
    #[test]
    fn the_counters_tell_a_skewed_joins_rows() {
        let mut x = 0x0477_u64;
        let mut next = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        let mut col = |heavy: u64| -> Vector<i64> { (0..20_000).map(|_| if next(10) < 3 { next(heavy) as i64 } else { 1_000 + next(5_000) as i64 }).collect() };
        let (a, b) = (col(20), col(40));
        let exact = |a: &[i64], b: &[i64]| {
            let mut m = std::collections::HashMap::new();
            for &v in b {
                *m.entry(v).or_insert(0.0) += 1.0;
            }
            a.iter().map(|v| m.get(v).copied().unwrap_or(0.0)).sum::<f64>()
        };
        let want = exact(&a, &b);
        let got = JoinCounts::of(a.iter().copied()).join(&JoinCounts::of(b.iter().copied()));
        assert!(got.tells(), "{got:?}");
        assert!((got.rows - want).abs() < 3.0 * got.err, "{got:?} against {want}");
        assert!((got.rows / want - 1.0).abs() < 0.2, "{got:?} against {want}");
        // Few values each: exact.
        let (s, t): (Vector<i64>, Vector<i64>) = ((0..3_000).map(|i| i % 7).collect(), (0..500).map(|i| i % 11).collect());
        let few = JoinCounts::of(s.iter().copied()).join(&JoinCounts::of(t.iter().copied()));
        assert_eq!(few, Joined { rows: exact(&s, &t), err: 0.0 });
        // Few against counters: within its error.
        let one = JoinCounts::of(s.iter().copied()).join(&JoinCounts::of(b.iter().copied()));
        assert!((one.rows - exact(&s, &b)).abs() <= 3.0 * one.err + 1e-9, "{one:?} against {}", exact(&s, &b));
    }

    /// Two columns of disjoint keys join no rows: the estimate is near none, and says it does not tell.
    #[test]
    fn disjoint_columns_estimate_near_no_rows() {
        let a = JoinCounts::of((0..20_000).map(|i| i % 3_000));
        let b = JoinCounts::of((0..20_000).map(|i| 10_000 + i % 3_000));
        let j = a.join(&b);
        assert!(j.rows < 3.0 * j.err, "{j:?}");
        assert!(!j.tells() || j.rows < 0.01 * (a.f2 * b.f2).sqrt(), "{j:?}");
    }
}
