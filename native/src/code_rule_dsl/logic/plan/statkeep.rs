//! Planner statistics kept by delta, in memory (decision 0096 §2–§3).
//!
//! A relation's statistics are what `syntax::measured` gives, `(column, rows, ndv, sq, sqd)`, but kept rather than
//! measured: a document's rows are counted in when it enters and out when it leaves or changes, so an edit costs
//! its own rows, not the repository's. Per column:
//!
//! ```text
//! sqd         Σ over (document, value) of count²      exact: a document's term is its own, subtracted whole
//! local ndv   Σ over documents of their distinct values  exact ndv while no value crosses documents (node columns)
//! exact       value → count, while ≤ 256 values         exact ndv and sq for few-valued columns
//! sketch      the 0096 sketch of value → count           ndv and sq within a small factor, for the rest
//! ```
//!
//! A row's document is its first column's high half (`>> 32`), as `syntax::measured` reads it. A column is a node
//! column, exact by sums, while every value names its row's document (a node `d << 32 | i`, or the document key
//! `d` itself): `crossing` counts the documents counted in that hold a value naming another.
//!
//! `delta` patches by rows that are not whole documents (a phase export's plus and minus): per-document sums are
//! then not additive, so the column's sums are given up and its exact map or sketch answers, with `sqd` as `sq`.

use rustc_hash::FxHashMap;

use super::sketch::Sketch;

/// Values a column's exact map holds at most; past it the map is dropped for the sketch (0096 §1).
const EXACT: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Column {
    sqd: i64,
    local_ndv: i64,
    /// Documents counted in that hold a value naming another document.
    crossing: i64,
    exact: Option<FxHashMap<i64, i64>>,
    sketch: Sketch,
}

/// One relation's statistics, kept by delta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct RelStats {
    columns: Vec<String>,
    rows: i64,
    /// Patched by rows that are not whole documents (`delta`): the per-document sums no longer hold.
    partial: bool,
    cols: Vec<Column>,
}

/// The document a row's first column names: as `syntax::measured` reads it.
fn doc32(first: i64) -> i64 {
    first >> 32
}

/// The document a location value names: a node (`key << 32 | index`) or a document key (as `memo::document_of`).
fn document_of(v: i64) -> i64 {
    if v >= 1 << 32 { v >> 32 } else { v }
}

/// Whether value `v` names document `d` (its first column's value `first`): a node of it, or its key.
fn names_doc(v: i64, first: i64) -> bool {
    let d = doc32(first);
    (d != 0 && v >> 32 == d) || v == document_of(first)
}

impl RelStats {
    /// No rows yet, of these columns.
    pub(in crate::code_rule_dsl) fn new(columns: Vec<String>) -> Self {
        let cols = columns.iter().map(|_| Column { exact: Some(FxHashMap::default()), ..Column::default() }).collect();
        RelStats { columns, rows: 0, partial: false, cols }
    }

    pub(in crate::code_rule_dsl) fn columns(&self) -> &[String] {
        &self.columns
    }

    pub(in crate::code_rule_dsl) fn rows(&self) -> i64 {
        self.rows
    }

    /// Whole documents' rows in (`sign` 1) or out (−1): `flat` row-major, `width` values a row (columns past it, in
    /// a row narrower than the relation, are not counted). Every row of each document they name must be in `flat`.
    pub(in crate::code_rule_dsl) fn documents(&mut self, flat: &[i64], width: usize, sign: i64) {
        if width == 0 || flat.is_empty() {
            return;
        }
        let n = flat.len() / width;
        self.rows += sign * n as i64;
        let mut counts: FxHashMap<(i64, i64), i64> = FxHashMap::default();
        let mut values: FxHashMap<i64, i64> = FxHashMap::default();
        let mut crossing: FxHashMap<i64, bool> = FxHashMap::default();
        for (c, col) in self.cols.iter_mut().enumerate().take(width) {
            counts.clear();
            values.clear();
            crossing.clear();
            for row in flat.chunks_exact(width) {
                let (first, v) = (row[0], row[c]);
                *counts.entry((doc32(first), v)).or_default() += 1;
                *values.entry(v).or_default() += 1;
                let named = names_doc(v, first);
                *crossing.entry(document_of(first)).or_default() |= !named;
            }
            col.sqd += sign * counts.values().map(|k| k * k).sum::<i64>();
            // Distinct values per document: (document, value) pairs, by the document a row names.
            let mut per_doc: FxHashMap<(i64, i64), ()> = FxHashMap::default();
            for row in flat.chunks_exact(width) {
                per_doc.insert((document_of(row[0]), row[c]), ());
            }
            col.local_ndv += sign * per_doc.len() as i64;
            col.crossing += sign * crossing.values().filter(|x| **x).count() as i64;
            Self::values(col, &values, sign);
        }
    }

    /// Rows in and out that are not whole documents (a phase export's delta), `plus` then `minus`, each row-major.
    pub(in crate::code_rule_dsl) fn delta(&mut self, plus: &[i64], minus: &[i64], width: usize) {
        if width == 0 {
            return;
        }
        self.partial = true;
        for (flat, sign) in [(plus, 1i64), (minus, -1)] {
            self.rows += sign * (flat.len() / width) as i64;
            let mut values: FxHashMap<i64, i64> = FxHashMap::default();
            for (c, col) in self.cols.iter_mut().enumerate().take(width) {
                values.clear();
                for row in flat.chunks_exact(width) {
                    *values.entry(row[c]).or_default() += 1;
                }
                Self::values(col, &values, sign);
            }
        }
    }

    fn values(col: &mut Column, values: &FxHashMap<i64, i64>, sign: i64) {
        for (&v, &k) in values {
            col.sketch.add(v, sign * k);
        }
        if let Some(exact) = &mut col.exact {
            for (&v, &k) in values {
                let e = exact.entry(v).or_default();
                *e += sign * k;
                if *e == 0 {
                    exact.remove(&v);
                }
            }
            if exact.len() > EXACT {
                col.exact = None;
            }
        }
    }

    /// The statistics, as `syntax::measured` gives them. A column not `held` (no rule reads it; it reads as 0) is
    /// stated as a key, as `columns_source`'s own measure states it.
    pub(in crate::code_rule_dsl) fn statistics(&self, held: &dyn Fn(&str) -> bool) -> Vec<(String, i64, i64, i64, i64)> {
        let n = self.rows.max(0);
        self.columns
            .iter()
            .zip(&self.cols)
            .map(|(name, col)| {
                if !held(name) || n == 0 {
                    return (name.clone(), n, n, n, n);
                }
                let (ndv, sq) = if !self.partial && col.crossing == 0 {
                    (col.local_ndv, col.sqd)
                } else if let Some(exact) = &col.exact {
                    (exact.len() as i64, exact.values().map(|k| k * k).sum())
                } else {
                    (col.sketch.ndv().round() as i64, col.sketch.sq().round() as i64)
                };
                let ndv = ndv.clamp(1, n);
                let sqd = if self.partial { sq } else { col.sqd };
                // Within its bounds: every row once at least (a key), every row one value at most, and no less
                // than within documents.
                let sq = sq.clamp(n.max(sqd.min(n * n)), n.saturating_mul(n));
                (name.clone(), n, ndv, sq, sqd.clamp(n, sq))
            })
            .collect()
    }

    /// Bytes `encode` takes, near enough (the exact maps and sketches dominate).
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.cols.iter().map(|c| 24 + c.exact.as_ref().map_or(0, |e| 16 * e.len()) + c.sketch.bytes()).sum::<usize>() + self.columns.iter().map(|c| c.len() + 4).sum::<usize>()
    }

    /// As stored in the wave's blob (0096 §2): a format byte, the columns, the rows, then per column its sums, its
    /// exact map (or none) and its sketch. Integers little-endian.
    pub(in crate::code_rule_dsl) fn encode(&self, out: &mut Vec<u8>) {
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
        out.push(1);
        int(out, self.columns.len() as i64);
        for c in &self.columns {
            int(out, c.len() as i64);
            out.extend_from_slice(c.as_bytes());
        }
        int(out, self.rows);
        out.push(u8::from(self.partial));
        for col in &self.cols {
            int(out, col.sqd);
            int(out, col.local_ndv);
            int(out, col.crossing);
            match &col.exact {
                None => int(out, -1),
                Some(e) => {
                    int(out, e.len() as i64);
                    let mut pairs: Vec<(&i64, &i64)> = e.iter().collect();
                    pairs.sort();
                    for (v, k) in pairs {
                        int(out, *v);
                        int(out, *k);
                    }
                }
            }
            let s = col.sketch.encode();
            int(out, s.len() as i64);
            out.extend_from_slice(&s);
        }
    }

    /// `encode`'s statistics back from `bytes` at `*at`, which moves past them; `None` if they are not.
    pub(in crate::code_rule_dsl) fn decode(bytes: &[u8], at: &mut usize) -> Option<RelStats> {
        fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Option<&'a [u8]> {
            let b = bytes.get(*at..*at + n)?;
            *at += n;
            Some(b)
        }
        let int = |at: &mut usize| -> Option<i64> { Some(i64::from_le_bytes(take(bytes, at, 8)?.try_into().ok()?)) };
        if take(bytes, at, 1)? != [1] {
            return None;
        }
        let k = usize::try_from(int(at)?).ok()?;
        let mut columns = Vec::with_capacity(k.min(1024));
        for _ in 0..k {
            let l = usize::try_from(int(at)?).ok()?;
            columns.push(String::from_utf8(take(bytes, at, l)?.to_vec()).ok()?);
        }
        let rows = int(at)?;
        let partial = take(bytes, at, 1)?[0] != 0;
        let mut cols = Vec::with_capacity(k);
        for _ in 0..k {
            let (sqd, local_ndv, crossing) = (int(at)?, int(at)?, int(at)?);
            let e = int(at)?;
            let exact = if e < 0 {
                None
            } else {
                let mut m = FxHashMap::default();
                for _ in 0..e {
                    let v = int(at)?;
                    m.insert(v, int(at)?);
                }
                Some(m)
            };
            let l = usize::try_from(int(at)?).ok()?;
            let sketch = Sketch::decode(take(bytes, at, l)?)?;
            cols.push(Column { sqd, local_ndv, crossing, exact, sketch });
        }
        Some(RelStats { columns, rows, partial, cols })
    }
}

#[cfg(test)]
mod tests {
    use super::RelStats;
    use crate::code_rule_dsl::facts::vtab::syntax;
    use crate::code_rule_dsl::logic::native::component::columns_source;

    /// Document `d`'s rows of `r(n, kind, name)`: `n` its nodes, `kind` few-valued, `name` many values shared
    /// across documents (Zipf-like), so each column takes another of the keeper's paths.
    fn doc(d: i64, rows: i64, salt: i64) -> Vec<i64> {
        (0..rows).flat_map(|i| [d << 32 | i, (i * 7 + salt) % 9, ((i * i + salt) % 997) / (1 + i % 5)]).collect()
    }

    fn measured(docs: &[Vec<i64>]) -> Vec<(String, i64, i64, i64, i64)> {
        let mut cols = vec![Vec::new(); 3];
        for d in docs {
            for row in d.chunks_exact(3) {
                for (c, v) in row.iter().enumerate() {
                    cols[c].push(*v);
                }
            }
        }
        syntax::measured(columns_source(vec!["n".into(), "kind".into(), "name".into()], cols.into_iter().map(Some).collect()).as_ref())
    }

    /// Kept by delta, the statistics are those of the documents counted in, whatever came and went before: node
    /// and few-valued columns exactly as measured, a many-valued one within the planner's margin. The blob reads
    /// back the same.
    #[test]
    fn statistics_kept_by_document_are_those_of_the_documents_counted_in() {
        let docs: Vec<Vec<i64>> = (1..=40).map(|d| doc(d, 50 + d * 13, d % 3)).collect();
        let mut kept = RelStats::new(vec!["n".into(), "kind".into(), "name".into()]);
        for d in &docs {
            kept.documents(d, 3, 1);
        }
        // Edits: a document changes, one leaves, one comes back changed again.
        let edited = doc(7, 400, 2);
        kept.documents(&docs[6], 3, -1);
        kept.documents(&edited, 3, 1);
        kept.documents(&docs[11], 3, -1);
        let mut now = docs.clone();
        now[6] = edited;
        now.remove(11);
        let (got, want) = (kept.statistics(&|_| true), measured(&now));
        assert_eq!(got[0], want[0], "a node column is exact");
        assert_eq!((got[1].1, got[1].2, got[1].3), (want[1].1, want[1].2, want[1].3), "a few-valued column is exact");
        let ratio = |a: i64, b: i64| a.max(b) as f64 / a.min(b).max(1) as f64;
        assert!(ratio(got[2].2, want[2].2) < 2.0 && ratio(got[2].3, want[2].3) < 2.0, "{:?} against {:?}", got[2], want[2]);
        assert_eq!(got[2].4, want[2].4, "sqd is exact");
        // Counted from scratch, the same counters: history does not matter.
        let mut fresh = RelStats::new(kept.columns().to_vec());
        for d in &now {
            fresh.documents(d, 3, 1);
        }
        assert_eq!(fresh, kept);
        let mut blob = Vec::new();
        kept.encode(&mut blob);
        let mut at = 0;
        assert_eq!(RelStats::decode(&blob, &mut at), Some(kept));
        assert_eq!(at, blob.len());
    }

    /// Patched by a delta of rows (not whole documents), the counts stay those of the rows: exact while few-valued.
    #[test]
    fn statistics_patched_by_a_delta_of_rows_count_the_rows() {
        let rows: Vec<i64> = (0..300).flat_map(|i| [i % 10, i % 4]).collect();
        let mut kept = RelStats::new(vec!["a".into(), "b".into()]);
        kept.delta(&rows, &[], 2);
        let (plus, minus): (Vec<i64>, Vec<i64>) = ((0..30).flat_map(|i| [i % 10, 9]).collect(), rows[..40].to_vec());
        kept.delta(&plus, &minus, 2);
        let mut now: Vec<i64> = rows[40..].to_vec();
        now.extend(&plus);
        let mut fresh = RelStats::new(kept.columns().to_vec());
        fresh.delta(&now, &[], 2);
        assert_eq!(fresh, kept);
        let got = kept.statistics(&|_| true);
        assert_eq!(got[1].1, 310);
        assert_eq!(got[1].2, 5, "b holds 0..4 and 9");
    }
}
