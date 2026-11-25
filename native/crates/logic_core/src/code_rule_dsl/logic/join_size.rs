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

/// A column's join-size counts, its counters and constants: the one linear sketch's (`rerail_blocks::sketch`, decision
/// 0678 step 10). Its sketch is their one writer, as the rows come; the planner's `JoinCounts` and the statistics' keys
/// (`bloom::Keys::Few`) read them where they lie (decision 0353: read in place, never copied).
pub use rerail_blocks::sketch::{BUCKETS, Counts, FEW as EXACT, bucket};

/// A column's counts as the planner reads them: its sketch's `Counts`, read where they lie, with its rows and sum
/// of squared rows a value (`F2`) as the statistics read them.
#[derive(Clone)]
pub struct JoinCounts {
    counts: std::sync::Arc<Counts>,
    pub rows: f64,
    pub f2: f64,
    /// Each value's rows, where its source counted them (`of_counts`), past `EXACT` values too: a point query is
    /// then exact, not its counter's (the few tiny documents of the annotations shared counters with the largest).
    exact: Option<std::sync::Arc<std::collections::HashMap<i64, u64>>>,
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
        JoinCounts { counts, rows: rows as f64, f2: f2 as f64, exact: None }
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

    /// A column's counts from each value's rows (`Source::column_counts`), as its rows one at a time would make them.
    pub fn of_counts(values: impl IntoIterator<Item = (i64, u64)>) -> JoinCounts {
        let counts = Counts::new();
        let (mut rows, mut f2) = (0u64, 0u64);
        let mut exact = std::collections::HashMap::new();
        for (v, n) in values {
            if n == 0 {
                continue;
            }
            counts.count_n(super::bloom::hash(v), n as i64);
            if !counts.add_few_n(v, n as i64) {
                counts.end_few();
            }
            rows += n;
            f2 += n * n;
            *exact.entry(v).or_default() += n;
        }
        JoinCounts { exact: Some(std::sync::Arc::new(exact)), ..JoinCounts::new(std::sync::Arc::new(counts), rows, f2) }
    }

    /// The rows of the values `keys` holds, where this column's rows are counted by value (`of_counts`): exact but
    /// for the keys' false positives.
    pub fn rows_held(&self, keys: &super::bloom::Keys) -> Option<f64> {
        Some(self.exact.as_ref()?.iter().filter(|(v, _)| keys.may_hold(**v)).map(|(_, &n)| n as f64).sum())
    }

    /// Value `v`'s rows: exact where its counts are kept, else its counter's, signed (unbiased; others of its
    /// bucket add their noise).
    fn point(&self, v: i64) -> f64 {
        if let Some(exact) = &self.exact {
            return exact.get(&v).map_or(0.0, |&n| n as f64);
        }
        match self.counts.few() {
            Some(mut few) => few.find(|e| e[0] == v).map_or(0.0, |e| e[1] as f64),
            None => {
                let (b, s) = bucket(super::bloom::hash(v));
                f64::from(self.counts.counter_at(b) * s)
            }
        }
    }

    /// The rows of the equi-join of this column with `other`: exact where both keep exact counts; else the AMS
    /// estimate (one side's values point-queried on the other's counters, where one keeps them), within the bounds
    /// any join of the two has (none, and `sqrt(F2 F2)` by Cauchy-Schwarz).
    pub fn join(&self, other: &JoinCounts) -> Joined {
        let bound = (self.f2 * other.f2).sqrt();
        let exact = |c: &JoinCounts, o: &JoinCounts| -> Option<f64> { Some(c.counts.few()?.map(|[v, n]| n as f64 * o.point(v)).sum()) };
        let told = |c: &JoinCounts| c.counts.few_len().is_some() || c.exact.is_some();
        let (rows, err) = if self.counts.few_len().is_some() && told(other) {
            (exact(self, other).unwrap_or(0.0), 0.0)
        } else if other.counts.few_len().is_some() && told(self) {
            (exact(other, self).unwrap_or(0.0), 0.0)
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
    use rerail_blocks::vector::Vector;

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

    /// A column counted by value (`of_counts`: 300 documents, a few large among many small) answers a point query
    /// exactly past `EXACT` values: joined with a few values, exactly; and the rows of the values a filter of more
    /// than `EXACT` keys holds are those values' rows, though their counters share buckets with the large ones.
    #[test]
    fn counts_by_value_price_a_few_small_values_exactly() {
        let docs: Vec<(i64, u64)> = (0..300).map(|d| (d, if d % 50 == 0 { 20_000 } else { 40 })).collect();
        let mine = JoinCounts::of_counts(docs.iter().copied());
        assert_eq!(mine.rows, docs.iter().map(|&(_, n)| n as f64).sum::<f64>());
        let small = JoinCounts::of([7, 8, 9]);
        let j = mine.join(&small);
        assert_eq!((j.rows, j.err), (120.0, 0.0));
        let keys = super::super::bloom::Keys::of((100..200).filter(|d| d % 50 != 0));
        let held = mine.rows_held(&keys).expect("counted by value");
        // Each held value's rows, and a large value's only where the filter's false positive takes one in.
        assert!((98.0 * 40.0..=98.0 * 40.0 + 2.0 * 20_000.0).contains(&held), "{held}");
        assert!(JoinCounts::of([1, 2]).rows_held(&keys).is_none(), "counted by rows, not by value");
    }
}
