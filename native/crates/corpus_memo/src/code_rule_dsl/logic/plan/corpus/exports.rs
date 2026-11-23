//! The exports the chunks hand to the cross-file part, held in memory and read in place.
//!
//! Each chunk adds its rows of each export once: a row another chunk added too, such as a file-local relation
//! without a location column, is kept once. The cross-file part binds each export as columns, the same as the base
//! relations held for it.
//!
//! This replaces the copy into the run's scratch database `x`: one `INSERT OR IGNORE INTO x.<rel> SELECT …` per
//! export per chunk, read back from `x` when the cross-file part started. On the Python pack that copy was 5.6 s of
//! about 89 s of CPU (166M SQLite VM steps, about 30 ns each).

use crate::code_rule_dsl::facts::rows::{IdSet, Life, Rows};
use crate::code_rule_dsl::facts::vtab::Source;
use hashbrown::HashTable;
use rustc_hash::FxHasher;
use std::collections::BTreeMap;
use std::hash::Hasher;
use std::sync::Arc;

/// One export's rows: each chunk's relation's parts, gathered where they lie as segments (`Rows::append_segment`, in
/// the chunk's blocks of kept rows, `Life::Kept`), with a set of row ids, hashed by the rows' values, that keeps each
/// row once. A row another chunk gave already, or dead in the chunk's relation, is dead here (`dead`): nothing is
/// written again (decision 0353; t52 measured the second write at 2.3 GB on 3,089 files). Rows of a source that is
/// not the engine's relation, and the memo's (`append_own`), are written in a tail of the export's own. A reader that
/// binds one relation in its block (the device's) merges the parts (`Rows::in_block`, 0353 row 9's copy, which goes
/// with the slab of step 6); the CPU's read them where they lie.
struct Held {
    rows: Rows,
    /// Rows not among the export's (bit `id`): a row given again, or dead in the relation it came from.
    dead: Vec<u64>,
    /// Its rows' ids by their values' hashes, in a block (`IdSet`, decision 0353): a heap table before (14.4 MB of
    /// "hold the export" at a 300-file run's peak on the M3).
    set: IdSet,
    /// Its columns' types, as the relations it holds give them (the engine's codes).
    types: Option<Vec<u8>>,
    /// The rows as a source, made when the cross-file part first reads them. No chunk adds after that.
    source: Option<Arc<dyn Source>>,
    /// Rows appended unchecked (`append_own`), not in `set`: each a document's own, which no other file's entry
    /// holds. Their ids, the location column that names their document, and the documents.
    unset: Vec<std::ops::Range<u32>>,
    at: Option<usize>,
    unset_docs: rustc_hash::FxHashSet<i64>,
    /// Let go after its last reader (`release`).
    released: bool,
}

impl Held {
    fn new(width: usize) -> Self {
        Held { rows: Rows::living(width, Life::Kept), dead: Vec::new(), set: IdSet::new(), types: None, source: None, unset: Vec::new(), at: None, unset_docs: Default::default(), released: false }
    }

    fn width(&self) -> usize {
        self.rows.width()
    }

    /// Row `id`'s value in column `c`, where it lies.
    #[inline]
    fn get(&self, id: u32, c: usize) -> i64 {
        self.rows.get(id, c)
    }

    fn is_dead(&self, id: u32) -> bool {
        self.dead.get(id as usize / 64).is_some_and(|w| w >> (id % 64) & 1 == 1)
    }

    fn kill(&mut self, id: u32) {
        let w = id as usize / 64;
        if self.dead.len() <= w {
            self.dead.resize(w + 1, 0);
        }
        self.dead[w] |= 1 << (id % 64);
    }

    fn row_hash(&self, id: u32) -> u64 {
        Held::hash((0..self.width()).map(|c| self.get(id, c)))
    }

    /// `row` written as the next row unless the set holds its like. Whether it was written.
    fn insert(&mut self, row: &[i64]) -> bool {
        let h = Held::hash(row.iter().copied());
        let (rows, width) = (&self.rows, self.width());
        if self.set.find(h, |i| (0..width).all(|c| rows.get(i, c) == row[c])).is_some() {
            return false;
        }
        let id = self.len() as u32;
        self.rows.extend_from_slice(row);
        let rows = &self.rows;
        self.set.insert_unique(h, id, |i| Held::hash((0..width).map(|c| rows.get(i, c))));
        true
    }

    /// Row `id` kept unless the set holds its like: then it is marked dead. Whether it was kept.
    fn keep_once(&mut self, id: u32) -> bool {
        let h = self.row_hash(id);
        let (rows, width) = (&self.rows, self.width());
        if self.set.find(h, |i| (0..width).all(|c| rows.get(i, c) == rows.get(id, c))).is_some() {
            self.kill(id);
            return false;
        }
        self.set.insert_unique(h, id, |i| Held::hash((0..width).map(|c| rows.get(i, c))));
        true
    }

    /// The rows appended unchecked put in the set: a row may now come again.
    fn set_all(&mut self) {
        for range in std::mem::take(&mut self.unset) {
            for id in range {
                let h = self.row_hash(id);
                let (rows, width) = (&self.rows, self.width());
                self.set.insert_unique(h, id, |i| Held::hash((0..width).map(|c| rows.get(i, c))));
            }
        }
        self.unset_docs.clear();
    }

    /// Rows held, dead ones too: the next row's id.
    fn len(&self) -> usize {
        self.rows.rows()
    }

    /// Rows held and not dead.
    fn live(&self) -> usize {
        self.len() - self.dead.iter().map(|w| w.count_ones() as usize).sum::<usize>()
    }

    /// Each live row's id.
    fn live_ids(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.len() as u32).filter(|&id| !self.is_dead(id))
    }

    fn hash(row: impl Iterator<Item = i64>) -> u64 {
        let mut h = FxHasher::default();
        for v in row {
            h.write_i64(v);
        }
        h.finish()
    }
}

/// Every export's rows, by relation.
#[derive(Default)]
pub struct Exports {
    rels: BTreeMap<String, Held>,
}

impl Exports {
    /// Adds the rows of `rows` (its first `width` columns) to export `rel`; how many were new. A relation the engine
    /// derived (`held_rows`) is read where it lies, its live rows not given before written once into the export's
    /// relation, as one batch's: nothing of the chunk's relation is kept. Other sources' rows are copied in.
    pub fn add(&mut self, rel: &str, width: usize, rows: &dyn Source) -> usize {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        debug_assert!(held.source.is_none(), "export {rel} added to after the cross-file part read it");
        let start = held.len() as u32;
        let in_place = crate::code_rule_dsl::logic::native::component::held_rows(rows)
            .filter(|(r, n, ..)| r.width() == width && r.rows() == *n);
        let given: Vec<bool> = match in_place {
            Some((r, n, dead, types)) => {
                held.types.get_or_insert_with(|| types.to_vec());
                // Each of the chunk's parts whole: taken as a segment of the export where it lies, not written again
                // (decision 0353, t52). A row another chunk gave already, or dead in the chunk's relation, is dead in
                // the export.
                let mut whole = true;
                r.each_run(0, n as u32, |p, lo, hi, _| whole &= lo == 0 && hi == p.len());
                if whole {
                    r.each_run(0, n as u32, |p, _, _, _| held.rows.append_segment(p));
                    let mut added = 0;
                    for i in 0..n {
                        let id = start + i as u32;
                        if dead.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1) {
                            held.kill(id);
                            continue;
                        }
                        // A row of a document whose own rows were appended unchecked may be one of them: they are set
                        // first.
                        if !held.unset.is_empty() && held.at.is_none_or(|at| held.unset_docs.contains(&super::memo::document_of(held.get(id, at)))) {
                            held.set_all();
                        }
                        if held.keep_once(id) {
                            added += 1;
                        }
                    }
                    return added;
                }
                // Room for every row at once: the tail grows once, not row by row, and pages no row reaches are
                // never touched.
                held.rows.reserve(n * width);
                let mut row = vec![0i64; width];
                let mut added = 0;
                r.each_run(0, n as u32, |p, lo, hi, first| {
                    for i in lo..hi {
                        let id = (first + i - lo) as usize;
                        if dead.get(id / 64).is_some_and(|w| w >> (id % 64) & 1 == 1) {
                            continue;
                        }
                        for (c, v) in row.iter_mut().enumerate() {
                            *v = p.get(i, c);
                        }
                        // A row of a document whose own rows were appended unchecked may be one of them: they are set
                        // first.
                        if !held.unset.is_empty() && held.at.is_none_or(|at| held.unset_docs.contains(&super::memo::document_of(row[at]))) {
                            held.set_all();
                        }
                        if held.insert(&row) {
                            added += 1;
                        }
                    }
                });
                return added;
            }
            None => {
                let mut row = vec![0i64; width];
                for r in 0..rows.len() {
                    for (c, v) in row.iter_mut().enumerate() {
                        *v = rows.value(r, c);
                    }
                    held.rows.extend_from_slice(&row);
                }
                vec![false; rows.len()]
            }
        };
        let mut added = 0;
        for (i, dead) in given.into_iter().enumerate() {
            let id = start + i as u32;
            if dead {
                held.kill(id);
                continue;
            }
            // A row of a document whose own rows were appended unchecked may be one of them: they are set first.
            if !held.unset.is_empty() && held.at.is_none_or(|at| held.unset_docs.contains(&super::memo::document_of(held.get(id, at)))) {
                held.set_all();
            }
            if held.keep_once(id) {
                added += 1;
            }
        }
        added
    }

    /// Appends rows (`columns`, `width` of them) to export `rel` without looking for them among those it holds, as
    /// `add` does: each row's column `at` names one of `documents`, whose own rows these are (a memo entry's rows of
    /// its own file), which no other file's rows hold, and no row is given twice. Rows that may be another file's
    /// too (a value document's, kept by each file that asked for it) go through `add`. A later `add` of a row of one
    /// of `documents` puts these in the set first, so a document that comes again is still kept once.
    pub fn append_own(&mut self, rel: &str, width: usize, at: usize, columns: Vec<Vec<i64>>, documents: impl IntoIterator<Item = i64>) -> usize {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        debug_assert!(held.source.is_none(), "export {rel} added to after the cross-file part read it");
        let n = columns.first().map_or(0, Vec::len);
        if n == 0 {
            return 0;
        }
        if held.at.is_some_and(|a| a != at) {
            held.set_all();
        }
        held.at = Some(at);
        let start = held.len() as u32;
        held.rows.reserve(n * width);
        let mut row = vec![0i64; width];
        for r in 0..n {
            for (v, column) in row.iter_mut().zip(&columns) {
                *v = column[r];
            }
            held.rows.extend_from_slice(&row);
        }
        held.unset.push(start..start + n as u32);
        held.unset_docs.extend(documents);
        n
    }

    /// How many rows export `rel` holds.
    pub fn len(&self, rel: &str) -> usize {
        self.rels.get(rel).map_or(0, |h| h.source.as_ref().map_or_else(|| h.live(), |s| s.len()))
    }

    /// Export `rel`'s rows so far, copied, as a source of columns `c0…`: for a reader that starts while chunks may
    /// still add (the cone of requests, decision 0065 §2a, whose loads go through the chunk path).
    pub fn snapshot(&self, rel: &str, width: usize) -> Arc<dyn Source> {
        let names = (0..width).map(|c| format!("c{c}")).collect();
        let values = match self.rels.get(rel) {
            Some(Held { source: Some(s), .. }) => return Arc::clone(s),
            Some(held) if held.width() >= width => {
                let mut cols = vec![Vec::with_capacity(held.live()); width];
                for id in held.live_ids() {
                    for (c, col) in cols.iter_mut().enumerate() {
                        col.push(held.get(id, c));
                    }
                }
                cols.into_iter().map(Some).collect()
            }
            _ => vec![Some(Vec::new()); width],
        };
        crate::code_rule_dsl::logic::native::component::columns_source(names, values)
    }

    /// Each of export `rel`'s rows held so far, read where it is held (`width` columns).
    pub(super) fn each_row(&self, rel: &str, width: usize, mut f: impl FnMut(Vec<i64>)) {
        self.visit(rel, width, |row| f(row.to_vec()));
    }

    /// Export `rel`'s rows held so far whose column `at` is a node of one of `documents` (`memo::document_of`):
    /// a load's new documents' rows, for the cone of requests that asked for them.
    pub fn rows_of(&self, rel: &str, width: usize, at: usize, documents: &std::collections::HashSet<i64>) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        if at < width {
            self.visit(rel, width, |row| {
                if documents.contains(&super::memo::document_of(row[at])) {
                    out.push(row.to_vec());
                }
            });
        }
        out
    }

    /// Visits held rows without a scratch SQL table or a second column store.
    pub fn visit(&self, rel: &str, width: usize, mut row: impl FnMut(&[i64])) {
        let Some(held) = self.rels.get(rel) else { return };
        let mut cells = vec![0; width];
        if let Some(source) = &held.source {
            for r in 0..source.len() {
                for (c, v) in cells.iter_mut().enumerate() {
                    *v = source.value(r, c);
                }
                row(&cells);
            }
        } else if held.width() >= width {
            for id in held.live_ids() {
                for (c, v) in cells.iter_mut().enumerate() {
                    *v = held.get(id, c);
                }
                row(&cells);
            }
        }
    }

    /// Export `rel` as a source of columns `c0…`: its rows, or none when no chunk added any. Made once. The rows
    /// move into it, where they lie, and the set that kept them distinct is dropped.
    pub fn source(&mut self, rel: &str, width: usize) -> Arc<dyn Source> {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        debug_assert!(!held.released, "export {rel} read after it was let go");
        if let Some(s) = &held.source {
            return Arc::clone(s);
        }
        let rows = std::mem::replace(&mut held.rows, Rows::living(width, Life::Kept)).sealed();
        let n = rows.rows();
        let dead = std::mem::take(&mut held.dead);
        held.set = IdSet::new();
        held.unset.clear();
        held.unset_docs.clear();
        let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
        let source = if n == 0 || rows.width() != width {
            crate::code_rule_dsl::logic::native::component::columns_source(names, vec![Some(Vec::new()); width])
        } else {
            let types = held.types.clone().unwrap_or_else(|| vec![b'i'; width]);
            crate::code_rule_dsl::logic::native::component::InRel::held(rows, n, types, dead).source(names)
        };
        held.source = Some(Arc::clone(&source));
        source
    }
}

impl Exports {
    /// Lets export `rel`'s rows go (Q74): the cross-file part read it for the last time. A later `source` is a
    /// mistake, caught in debug builds.
    pub fn release(&mut self, rel: &str) {
        if let Some(held) = self.rels.get_mut(rel) {
            let width = held.width().max(held.source.as_ref().map_or(0, |s| s.columns().len()));
            *held = Held::new(width);
            held.released = true;
        }
    }

    /// Export `rel`'s rows are `source` (the kept inputs' rows, decision 0117; the cone's rows less a load taken
    /// back): what a chunk added this run is dropped, as `source` holds it.
    pub fn install(&mut self, rel: &str, width: usize, source: Arc<dyn Source>) {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        held.rows = Rows::living(width, Life::Kept);
        held.dead.clear();
        held.set = IdSet::new();
        held.source = Some(source);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn an_export_keeps_each_row_once_across_chunks() {
        let chunk = |rows: Vec<[i64; 2]>| {
            let names = vec!["c0".to_owned(), "c1".to_owned()];
            crate::code_rule_dsl::logic::native::component::columns_source(names, (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect())
        };
        let mut exports = Exports::default();
        assert_eq!(exports.add("e", 2, chunk(vec![[1, 2], [3, 4], [1, 2]]).as_ref()), 2);
        assert_eq!(exports.add("e", 2, chunk(vec![[3, 4], [5, 6]]).as_ref()), 1);
        let s = exports.source("e", 2);
        let rows: Vec<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
        assert_eq!(rows, vec![(1, 2), (3, 4), (5, 6)]);
        // An export no chunk had rows of is bound empty.
        assert_eq!(exports.source("none", 3).len(), 0);
    }

    /// t52 (amending decision 0384 step 1): an export holds the chunks' relations' parts where they lie, as
    /// segments; nothing is written again. A row another chunk gave first, or one dead in its relation, is dead in the
    /// export, and its readers read its live rows. Rows of a source that is no engine relation go in a tail of the
    /// export's own, never into a chunk's relation.
    #[test]
    fn an_export_reads_the_chunks_rows_where_they_lie() {
        use crate::code_rule_dsl::logic::native::component::{InRel, held_rows};
        let names = || vec!["c0".to_owned(), "c1".to_owned()];
        let a = InRel::from_flat(vec![1, 2, 3, 4, 7, 8], 3, vec![b'i', b'i']).source(names());
        let mut dead = vec![0u64];
        dead[0] |= 1 << 2;
        // Rows [3, 4] (given by `a` too), [5, 6], and [9, 9], dead in its relation.
        let b = InRel::held(Rows::frozen(2, vec![3, 4, 5, 6, 9, 9], 3), 3, vec![b'i', b'i'], dead).source(names());
        // Not an engine relation: its rows are written, [5, 6] again and [11, 12].
        let c = crate::code_rule_dsl::logic::native::component::columns_source(names(), vec![Some(vec![5, 11]), Some(vec![6, 12])]);
        let mut exports = Exports::default();
        assert_eq!(exports.add("e", 2, a.as_ref()), 3);
        assert_eq!(exports.add("e", 2, b.as_ref()), 1);
        assert_eq!(exports.add("e", 2, c.as_ref()), 1);
        {
            let held = &exports.rels["e"];
            let (ra, ..) = held_rows(a.as_ref()).expect("rows");
            let (rb, ..) = held_rows(b.as_ref()).expect("rows");
            let theirs = |r: &Rows| held.rows.parts().filter(|p| r.parts().any(|q| std::ptr::eq(p.rel(), q.rel()))).count();
            assert_eq!((theirs(ra), theirs(rb)), (1, 1), "each chunk's part held where it lies");
            assert_eq!((ra.rows(), rb.rows()), (3, 3), "nothing written into a chunk's relation");
            assert_eq!(held.rows.parts().count(), 3, "the two parts and a tail of its own");
            assert_eq!((held.rows.rows(), held.live()), (8, 5), "the repeats and the dead row dead, not written");
        }
        assert_eq!(exports.len("e"), 5);
        let s = exports.source("e", 2);
        let rows: Vec<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
        assert_eq!(rows, vec![(1, 2), (3, 4), (7, 8), (5, 6), (11, 12)]);
    }

    #[test]
    fn a_documents_own_rows_are_appended_unchecked_and_still_kept_once() {
        let chunk = |rows: Vec<[i64; 2]>| {
            let names = vec!["c0".to_owned(), "c1".to_owned()];
            crate::code_rule_dsl::logic::native::component::columns_source(names, (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect())
        };
        let node = |doc: i64, i: i64| (doc << 32) | i;
        let mut exports = Exports::default();
        // Document 1's own rows, then document 2's, appended unchecked.
        assert_eq!(exports.append_own("e", 2, 0, vec![vec![node(1, 0), node(1, 1)], vec![7, 8]], [1]), 2);
        assert_eq!(exports.append_own("e", 2, 0, vec![vec![node(2, 0)], vec![9]], [2]), 1);
        // Another document's rows are checked against the set alone; a shared row is kept once.
        assert_eq!(exports.add("e", 2, chunk(vec![[node(3, 0), 1], [node(3, 0), 1]]).as_ref()), 1);
        // A row of document 1 again: the unchecked rows are set first, so it is found.
        assert_eq!(exports.add("e", 2, chunk(vec![[node(1, 1), 8], [node(1, 2), 8]]).as_ref()), 1);
        let s = exports.source("e", 2);
        let mut rows: Vec<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
        rows.sort();
        assert_eq!(rows, vec![(node(1, 0), 7), (node(1, 1), 8), (node(1, 2), 8), (node(2, 0), 9), (node(3, 0), 1)]);
    }

    #[test]
    fn a_documents_own_rows_and_others_given_again_are_kept_once() {
        let chunk = |rows: Vec<[i64; 2]>| {
            let names = vec!["c0".to_owned(), "c1".to_owned()];
            crate::code_rule_dsl::logic::native::component::columns_source(names, (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect())
        };
        let node = |doc: i64, i: i64| (doc << 32) | i;
        let mut exports = Exports::default();
        // Document 1's own rows, then document 2's.
        exports.add("e", 2, chunk(vec![[node(1, 0), 7], [node(1, 1), 8]]).as_ref());
        exports.add("e", 2, chunk(vec![[node(2, 0), 9]]).as_ref());
        // Another document's row twice, and a row of document 1 again.
        exports.add("e", 2, chunk(vec![[node(3, 0), 1], [node(3, 0), 1]]).as_ref());
        exports.add("e", 2, chunk(vec![[node(1, 1), 8], [node(1, 2), 8]]).as_ref());
        let mut docs = std::collections::HashSet::new();
        docs.insert(1);
        assert_eq!(exports.rows_of("e", 2, 0, &docs), vec![vec![node(1, 0), 7], vec![node(1, 1), 8], vec![node(1, 2), 8]]);
        let s = exports.source("e", 2);
        let mut rows: Vec<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
        rows.sort();
        assert_eq!(rows, vec![(node(1, 0), 7), (node(1, 1), 8), (node(1, 2), 8), (node(2, 0), 9), (node(3, 0), 1)]);
    }
}
