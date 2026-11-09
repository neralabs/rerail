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
//! `delta` patches by rows that are not whole documents (a phase export's plus and minus, a maintained relation's
//! rows gone and new): per-document sums are then not additive, so the column's sums are given up and its exact map
//! or sketch answers, with `sqd` as `sq`, or as a sketch of the (document, value) pairs' counts where one is kept
//! (`paired`, decision 0096 step 4): the sketch is linear, so a row's delete takes out what its insert put in.

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
    /// `paired`: the 0096 sketch of each (document, value) pair's count, whose `sq` is the column's `sqd`.
    pairs: Option<Sketch>,
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

/// The key a (document, value) pair is counted under in a column's `pairs` sketch: the sketch hashes it again.
fn pair(doc: i64, v: i64) -> i64 {
    v ^ doc.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64).rotate_left(29)
}

/// The group `syntax::measured` sums a row's `sqd` within: its first column's document, all document keys (`doc32`
/// 0) one group.
fn group(first: i64) -> i64 {
    if doc32(first) == 0 { 0 } else { document_of(first) }
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

    /// `new`, keeping each column's (document, value) pairs sketched too: for a relation patched by rows that are
    /// not whole documents (a maintained relation's delta), whose `sqd` the per-document sums no longer give.
    pub(in crate::code_rule_dsl) fn paired(columns: Vec<String>) -> Self {
        let mut s = Self::new(columns);
        for c in &mut s.cols {
            c.pairs = Some(Sketch::default());
        }
        s
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
        self.rows_of(flat, width, sign, None)
    }

    /// Rows in or out, as `documents`, where a row's document is the one its column `at` names (a node,
    /// `key << 32 | index`), or one document for all when `at` is `None`: as the chunks' totals read an export's
    /// rows (its first node column), whose first column may hold a value many documents share.
    pub(in crate::code_rule_dsl) fn rows_by(&mut self, flat: &[i64], width: usize, sign: i64, at: Option<usize>) {
        self.rows_of(flat, width, sign, Some(at))
    }

    fn rows_of(&mut self, flat: &[i64], width: usize, sign: i64, at: Option<Option<usize>>) {
        if width == 0 || flat.is_empty() {
            return;
        }
        // A row's document (as `doc32` and `document_of` read its first column, else by `at`), and whether `v` names it.
        let of = |row: &[i64]| match at {
            None => (doc32(row[0]), document_of(row[0])),
            Some(at) => {
                let d = at.map_or(-1, |c| row[c] >> 32);
                (d, d)
            }
        };
        let names = |v: i64, row: &[i64]| match at {
            None => names_doc(v, row[0]),
            Some(at) => at.is_some_and(|c| v >> 32 == row[c] >> 32),
        };
        let n = flat.len() / width;
        self.rows += sign * n as i64;
        // One pass a column: each row's value counted in its document's map (a document's rows mostly come together,
        // so its map is looked up once a run); the sums are read from the maps. A document is its row's
        // `document_of`; a value's count is split by whether the row's `doc32` is 0, as `syntax::measured` groups
        // sqd by `doc32` (all rows whose first column is a document key are one group there).
        type Doc = (FxHashMap<i64, (i64, i64)>, bool);
        let mut docs: FxHashMap<i64, Doc> = FxHashMap::default();
        let mut values: FxHashMap<i64, i64> = FxHashMap::default();
        let mut zero: FxHashMap<i64, i64> = FxHashMap::default();
        for (c, col) in self.cols.iter_mut().enumerate().take(width) {
            docs.clear();
            values.clear();
            zero.clear();
            let mut at: Option<(i64, Doc)> = None;
            for row in flat.chunks_exact(width) {
                let (v, (d32, doc)) = (row[c], of(row));
                if at.as_ref().is_none_or(|(d, _)| *d != doc) {
                    if let Some((d, m)) = at.take() {
                        docs.insert(d, m);
                    }
                    at = Some((doc, docs.remove(&doc).unwrap_or_default()));
                }
                let (_, (m, crosses)) = at.as_mut().expect("set");
                let e = m.entry(v).or_default();
                if d32 == 0 { e.0 += 1 } else { e.1 += 1 }
                *crosses |= !names(v, row);
            }
            if let Some((d, m)) = at.take() {
                docs.insert(d, m);
            }
            let (mut sqd, mut local, mut crossing) = (0i64, 0i64, 0i64);
            for (&d, (m, crosses)) in &docs {
                local += m.len() as i64;
                crossing += i64::from(*crosses);
                for (&v, &(small, large)) in m {
                    *values.entry(v).or_default() += small + large;
                    sqd += large * large;
                    if large > 0
                        && let Some(p) = &mut col.pairs
                    {
                        p.add(pair(d, v), sign * large);
                    }
                    if small > 0 {
                        *zero.entry(v).or_default() += small;
                    }
                }
            }
            sqd += zero.values().map(|k| k * k).sum::<i64>();
            if let Some(p) = &mut col.pairs {
                for (&v, &k) in &zero {
                    p.add(pair(0, v), sign * k);
                }
            }
            col.sqd += sign * sqd;
            col.local_ndv += sign * local;
            col.crossing += sign * crossing;
            Self::values(col, &values, sign);
        }
    }

    /// Rows in and out that are not whole documents (a phase export's delta), `plus` then `minus`, each row-major.
    pub(in crate::code_rule_dsl) fn delta(&mut self, plus: &[i64], minus: &[i64], width: usize) {
        if width == 0 {
            return;
        }
        self.partial = true;
        // Out before in: a value whose rows are replaced never counts past them, so an exact map is not given up on
        // the way.
        for (flat, sign) in [(minus, -1i64), (plus, 1)] {
            self.rows += sign * (flat.len() / width) as i64;
            let mut values: FxHashMap<i64, i64> = FxHashMap::default();
            let mut pairs: FxHashMap<(i64, i64), i64> = FxHashMap::default();
            for (c, col) in self.cols.iter_mut().enumerate().take(width) {
                values.clear();
                for row in flat.chunks_exact(width) {
                    *values.entry(row[c]).or_default() += 1;
                }
                Self::values(col, &values, sign);
                if let Some(p) = &mut col.pairs {
                    pairs.clear();
                    for row in flat.chunks_exact(width) {
                        *pairs.entry((group(row[0]), row[c])).or_default() += 1;
                    }
                    for (&(d, v), &k) in &pairs {
                        p.add(pair(d, v), sign * k);
                    }
                }
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
                let sqd = match (&col.pairs, self.partial) {
                    (_, false) => col.sqd,
                    (Some(p), true) => p.sq().round() as i64,
                    (None, true) => sq,
                };
                // Within its bounds: every row once at least (a key), every row one value at most, and no less
                // than within documents.
                let sq = sq.clamp(n.max(sqd.min(n * n)), n.saturating_mul(n));
                (name.clone(), n, ndv, sq, sqd.clamp(n, sq))
            })
            .collect()
    }

    /// Column `c`'s count of each value, while it has few enough to keep them exactly (`EXACT`).
    pub(in crate::code_rule_dsl) fn exact(&self, c: usize) -> Option<impl Iterator<Item = (i64, i64)> + '_> {
        self.cols.get(c)?.exact.as_ref().map(|e| e.iter().map(|(v, n)| (*v, *n)))
    }

    /// Bytes `encode` takes, near enough (the exact maps and sketches dominate).
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.cols.iter().map(|c| 24 + c.exact.as_ref().map_or(0, |e| 16 * e.len()) + c.sketch.bytes() + c.pairs.as_ref().map_or(0, Sketch::bytes)).sum::<usize>() + self.columns.iter().map(|c| c.len() + 4).sum::<usize>()
    }

    /// As stored in the wave's blob (0096 §2): a format byte, the columns, the rows, then per column its sums, its
    /// exact map (or none) and its sketch. Integers little-endian.
    pub(in crate::code_rule_dsl) fn encode(&self, out: &mut Vec<u8>) {
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
        out.push(2);
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
            match &col.pairs {
                None => int(out, -1),
                Some(p) => {
                    let s = p.encode();
                    int(out, s.len() as i64);
                    out.extend_from_slice(&s);
                }
            }
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
        if take(bytes, at, 1)? != [2] {
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
            let pairs = match int(at)? {
                l if l < 0 => None,
                l => Some(Sketch::decode(take(bytes, at, usize::try_from(l).ok()?)?)?),
            };
            cols.push(Column { sqd, local_ndv, crossing, exact, sketch, pairs });
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
    /// A relation counted whole and then patched by rows that are not whole documents (a maintained relation's
    /// delta) keeps its within-document sum by its pairs' sketch, within the planner's margin, where an unpaired one
    /// reads it as `sq`; its rows and its few-valued column stay exact.
    #[test]
    fn a_paired_relation_patched_by_rows_keeps_its_sum_within_documents() {
        let docs: Vec<Vec<i64>> = (1..=40).map(|d| doc(d, 50 + d * 13, d % 3)).collect();
        let names = vec!["n".to_owned(), "kind".into(), "name".into()];
        let (mut paired, mut plain) = (RelStats::paired(names.clone()), RelStats::new(names));
        let all: Vec<i64> = docs.concat();
        paired.documents(&all, 3, 1);
        plain.documents(&all, 3, 1);
        // Rows out of some documents, and others into them: not whole documents.
        let removed: Vec<i64> = docs[3][..30].iter().chain(&docs[20][..60]).copied().collect();
        let added: Vec<i64> = doc(9, 70, 1)[3 * 50..].iter().chain(&doc(41, 30, 0)).copied().collect();
        paired.delta(&added, &removed, 3);
        plain.delta(&added, &removed, 3);
        let mut now = docs.clone();
        now[3].drain(..30);
        now[20].drain(..60);
        now.push(added);
        let (got, flat, want) = (paired.statistics(&|_| true), plain.statistics(&|_| true), measured(&now));
        let ratio = |a: i64, b: i64| a.max(b) as f64 / a.min(b).max(1) as f64;
        for c in 0..3 {
            assert_eq!(got[c].1, want[c].1, "rows are exact");
            assert!(ratio(got[c].4, want[c].4) < 1.5, "column {c}: sqd {:?} against {:?}", got[c], want[c]);
            assert!(ratio(got[c].2, want[c].2) < 2.0 && ratio(got[c].3, want[c].3) < 2.0, "column {c}: {:?} against {:?}", got[c], want[c]);
        }
        assert_eq!((got[1].2, got[1].3), (want[1].2, want[1].3), "a few-valued column is exact");
        assert!(flat[1].4 == flat[1].3 && flat[1].4 > want[1].4, "unpaired, sqd reads as sq");
        let mut blob = Vec::new();
        paired.encode(&mut blob);
        assert_eq!(RelStats::decode(&blob, &mut 0), Some(paired));
    }

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

    /// The one-pass count gives the counters the four passes it replaced gave: rows naming their document by its
    /// key or by its nodes, documents whose rows come apart, values crossing documents, with and without `at`.
    #[test]
    fn one_pass_counts_as_the_four_passes_did() {
        use rustc_hash::FxHashMap;
        // The old count, kept here as the reference.
        fn old(stats: &mut RelStats, flat: &[i64], width: usize, sign: i64, at: Option<Option<usize>>) {
            let of = |row: &[i64]| match at {
                None => (super::doc32(row[0]), super::document_of(row[0])),
                Some(at) => {
                    let d = at.map_or(-1, |c| row[c] >> 32);
                    (d, d)
                }
            };
            let names = |v: i64, row: &[i64]| match at {
                None => super::names_doc(v, row[0]),
                Some(at) => at.is_some_and(|c| v >> 32 == row[c] >> 32),
            };
            stats.rows += sign * (flat.len() / width) as i64;
            for (c, col) in stats.cols.iter_mut().enumerate().take(width) {
                let (mut counts, mut values, mut crossing, mut per_doc) = (FxHashMap::<(i64, i64), i64>::default(), FxHashMap::<i64, i64>::default(), FxHashMap::<i64, bool>::default(), FxHashMap::<(i64, i64), ()>::default());
                for row in flat.chunks_exact(width) {
                    let (v, (d32, dof)) = (row[c], of(row));
                    *counts.entry((d32, v)).or_default() += 1;
                    *values.entry(v).or_default() += 1;
                    *crossing.entry(dof).or_default() |= !names(v, row);
                    per_doc.insert((dof, v), ());
                }
                col.sqd += sign * counts.values().map(|k| k * k).sum::<i64>();
                col.local_ndv += sign * per_doc.len() as i64;
                col.crossing += sign * crossing.values().filter(|x| **x).count() as i64;
                RelStats::values(col, &values, sign);
            }
        }
        let mut flat = Vec::new();
        for i in 0..600i64 {
            let d = 1 + i % 7;
            let first = if i % 5 == 0 { d } else { d << 32 | i };
            let other = if i % 11 == 0 { (d + 1) << 32 | i } else { d << 32 | (i / 3) };
            flat.extend([first, i % 4, other]);
        }
        for at in [None, Some(None), Some(Some(0)), Some(Some(2))] {
            let (mut a, mut b) = (RelStats::new(vec!["x".into(), "k".into(), "y".into()]), RelStats::new(vec!["x".into(), "k".into(), "y".into()]));
            a.rows_of(&flat, 3, 1, at);
            old(&mut b, &flat, 3, 1, at);
            assert_eq!(a, b, "at {at:?}");
            a.rows_of(&flat[..300], 3, -1, at);
            old(&mut b, &flat[..300], 3, -1, at);
            assert_eq!(a, b, "at {at:?}, counted out");
        }
    }
}
