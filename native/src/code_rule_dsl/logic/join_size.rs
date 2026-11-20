//! A join's size from its columns' count sketches (decision 0477): the AMS estimate (Alon, Matias and Szegedy; Alon,
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

/// The counters a column keeps.
pub const BUCKETS: usize = 256;
/// The values a column keeps exact counts of, at most.
pub const EXACT: usize = 64;

/// A key's counter and its sign there, from its hash (`bloom::hash`).
#[inline(always)]
pub fn bucket(h: u64) -> (usize, i32) {
    (((h >> 8) & 0xff) as usize, if h & 1 == 0 { 1 } else { -1 })
}

/// A column's counts as the planner reads them: its counters, its rows and sum of squared rows a value (`F2`), and
/// its values' exact counts while it has at most `EXACT`.
#[derive(Clone, Debug)]
pub struct JoinCounts {
    counters: Box<[i32; BUCKETS]>,
    /// By value.
    exact: Option<Box<[(i64, u32)]>>,
    pub rows: f64,
    pub f2: f64,
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
    /// From a sketch's counters, its exact counts where it keeps few, its rows and its `F2`.
    pub fn new(counters: &[i32; BUCKETS], exact: Option<&[(i64, u32)]>, rows: u64, f2: u64) -> JoinCounts {
        let exact = exact.filter(|e| e.len() <= EXACT).map(|e| {
            let mut e: Box<[(i64, u32)]> = e.into();
            e.sort_unstable_by_key(|t| t.0);
            e
        });
        JoinCounts { counters: Box::new(*counters), exact, rows: rows as f64, f2: f2 as f64 }
    }

    /// A column's counts, from its values (the sketch's adds, for tests and columns the engine does not sketch).
    pub fn of(values: impl IntoIterator<Item = i64>) -> JoinCounts {
        let mut counters = [0i32; BUCKETS];
        let mut counts: std::collections::HashMap<i64, u32> = std::collections::HashMap::new();
        for v in values {
            let (b, s) = bucket(super::bloom::hash(v));
            counters[b] += s;
            *counts.entry(v).or_default() += 1;
        }
        let rows = counts.values().map(|&n| u64::from(n)).sum();
        let f2 = counts.values().map(|&n| u64::from(n).pow(2)).sum();
        let exact: Vec<(i64, u32)> = counts.into_iter().collect();
        JoinCounts::new(&counters, Some(&exact), rows, f2)
    }

    /// Value `v`'s rows: exact where its counts are kept, else its counter's, signed (unbiased; others of its
    /// bucket add their noise).
    fn point(&self, v: i64) -> f64 {
        match &self.exact {
            Some(e) => e.binary_search_by_key(&v, |t| t.0).map_or(0.0, |i| f64::from(e[i].1)),
            None => {
                let (b, s) = bucket(super::bloom::hash(v));
                f64::from(self.counters[b] * s)
            }
        }
    }

    /// The rows of the equi-join of this column with `other`: exact where both keep exact counts; else the AMS
    /// estimate (one side's values point-queried on the other's counters, where one keeps them), within the bounds
    /// any join of the two has (none, and `sqrt(F2 F2)` by Cauchy-Schwarz).
    pub fn join(&self, other: &JoinCounts) -> Joined {
        let bound = (self.f2 * other.f2).sqrt();
        let (rows, err) = match (&self.exact, &other.exact) {
            (Some(a), Some(_)) => (a.iter().map(|&(v, n)| f64::from(n) * other.point(v)).sum(), 0.0),
            (Some(e), None) | (None, Some(e)) => {
                let o = if self.exact.is_some() { other } else { self };
                let j: f64 = e.iter().map(|&(v, n)| f64::from(n) * o.point(v)).sum();
                (j, (bound * bound / BUCKETS as f64).sqrt())
            }
            (None, None) => {
                let j: f64 = self.counters.iter().zip(other.counters.iter()).map(|(&a, &b)| f64::from(a) * f64::from(b)).sum();
                (j, ((bound * bound + j * j) / BUCKETS as f64).sqrt())
            }
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
        let mut col = |heavy: u64| -> Vec<i64> { (0..20_000).map(|_| if next(10) < 3 { next(heavy) as i64 } else { 1_000 + next(5_000) as i64 }).collect() };
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
        let (s, t): (Vec<i64>, Vec<i64>) = ((0..3_000).map(|i| i % 7).collect(), (0..500).map(|i| i % 11).collect());
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
