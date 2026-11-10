//! The exports the chunks hand to the cross-file part, held in memory and read in place.
//!
//! Each chunk adds its rows of each export once: a row another chunk added too, such as a file-local relation
//! without a location column, is kept once. The cross-file part binds each export as columns, the same as the base
//! relations held for it.
//!
//! This replaces the copy into the run's scratch database `x`: one `INSERT OR IGNORE INTO x.<rel> SELECT …` per
//! export per chunk, read back from `x` when the cross-file part started. On the Python pack that copy was 5.6 s of
//! about 89 s of CPU (166M SQLite VM steps, about 30 ns each).

use crate::code_rule_dsl::facts::vtab::Source;
use hashbrown::HashTable;
use rustc_hash::FxHasher;
use std::collections::BTreeMap;
use std::hash::Hasher;
use std::sync::Arc;

/// One export's rows: columns, and a set of row ids, hashed by the rows' values, that keeps each row once.
struct Held {
    columns: Vec<Vec<i64>>,
    set: HashTable<u32>,
    /// The rows as a source, made when the cross-file part first reads them. No chunk adds after that.
    source: Option<Arc<dyn Source>>,
    /// Rows appended unchecked (`append_own`), not in `set`: each a document's own, which no other file's entry
    /// holds. Their ids, the location column that names their document, and the documents.
    unset: Vec<std::ops::Range<u32>>,
    at: Option<usize>,
    unset_docs: rustc_hash::FxHashSet<i64>,
}

impl Held {
    fn new(width: usize) -> Self {
        Held { columns: vec![Vec::new(); width], set: HashTable::new(), source: None, unset: Vec::new(), at: None, unset_docs: Default::default() }
    }

    /// The rows appended unchecked put in the set: a row may now come again.
    fn set_all(&mut self) {
        let columns = &self.columns;
        for range in std::mem::take(&mut self.unset) {
            for id in range {
                let h = Held::hash(columns.iter().map(|col| col[id as usize]));
                self.set.insert_unique(h, id, |&i| Held::hash(columns.iter().map(|col| col[i as usize])));
            }
        }
        self.unset_docs.clear();
    }

    fn len(&self) -> usize {
        self.columns.first().map_or(0, Vec::len)
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
pub(super) struct Exports {
    rels: BTreeMap<String, Held>,
}

impl Exports {
    /// Adds the rows of `rows` (its first `width` columns) to export `rel`; how many were new.
    pub(super) fn add(&mut self, rel: &str, width: usize, rows: &dyn Source) -> usize {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        debug_assert!(held.source.is_none(), "export {rel} added to after the cross-file part read it");
        let mut added = 0;
        let mut row = vec![0i64; width];
        for r in 0..rows.len() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = rows.value(r, c);
            }
            // A row of a document whose own rows were appended unchecked may be one of them: they are set first.
            if !held.unset.is_empty() && held.at.is_none_or(|at| held.unset_docs.contains(&super::memo::document_of(row[at]))) {
                held.set_all();
            }
            let h = Held::hash(row.iter().copied());
            let columns = &held.columns;
            let same = |&id: &u32| row.iter().enumerate().all(|(c, v)| columns[c][id as usize] == *v);
            if held.set.find(h, same).is_some() {
                continue;
            }
            let id = held.len() as u32;
            for (c, v) in row.iter().enumerate() {
                held.columns[c].push(*v);
            }
            let columns = &held.columns;
            held.set.insert_unique(h, id, |&i| Held::hash(columns.iter().map(|col| col[i as usize])));
            added += 1;
        }
        added
    }

    /// Appends rows (`columns`, `width` of them) to export `rel` without looking for them among those it holds, as
    /// `add` does: each row's column `at` names one of `documents`, whose own rows these are (a memo entry's rows of
    /// its own file), which no other file's rows hold, and no row is given twice. Rows that may be another file's
    /// too (a value document's, kept by each file that asked for it) go through `add`. A later `add` of a row of one
    /// of `documents` puts these in the set first, so a document that comes again is still kept once.
    pub(super) fn append_own(&mut self, rel: &str, width: usize, at: usize, columns: Vec<Vec<i64>>, documents: impl IntoIterator<Item = i64>) -> usize {
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
        for (to, from) in held.columns.iter_mut().zip(columns) {
            to.extend(from);
        }
        held.unset.push(start..start + n as u32);
        held.unset_docs.extend(documents);
        n
    }

    /// How many rows export `rel` holds.
    pub(super) fn len(&self, rel: &str) -> usize {
        self.rels.get(rel).map_or(0, |h| h.source.as_ref().map_or_else(|| h.len(), |s| s.len()))
    }

    /// Export `rel`'s rows so far, copied, as a source of columns `c0…`: for a reader that starts while chunks may
    /// still add (the cone of requests, decision 0065 §2a, whose loads go through the chunk path).
    pub(super) fn snapshot(&self, rel: &str, width: usize) -> Arc<dyn Source> {
        let names = (0..width).map(|c| format!("c{c}")).collect();
        let values = match self.rels.get(rel) {
            Some(held) if held.source.is_none() && !held.columns.is_empty() => held.columns.iter().cloned().map(Some).collect(),
            Some(Held { source: Some(s), .. }) => return Arc::clone(s),
            _ => vec![Some(Vec::new()); width],
        };
        crate::code_rule_dsl::logic::native::component::columns_source(names, values)
    }

    /// Each of export `rel`'s rows held so far, read where it is held (`width` columns).
    pub(super) fn each_row(&self, rel: &str, width: usize, mut f: impl FnMut(Vec<i64>)) {
        match self.rels.get(rel) {
            Some(Held { source: Some(s), .. }) => (0..s.len()).for_each(|r| f((0..width).map(|c| s.value(r, c)).collect())),
            Some(Held { source: None, columns, .. }) if columns.len() >= width => {
                (0..columns.first().map_or(0, Vec::len)).for_each(|r| f(columns[..width].iter().map(|col| col[r]).collect()))
            }
            _ => {}
        }
    }

    /// Export `rel`'s rows held so far whose column `at` is a node of one of `documents` (`memo::document_of`):
    /// a load's new documents' rows, for the cone of requests that asked for them.
    pub(super) fn rows_of(&self, rel: &str, width: usize, at: usize, documents: &std::collections::HashSet<i64>) -> Vec<Vec<i64>> {
        let columns: &[Vec<i64>] = match self.rels.get(rel) {
            Some(Held { source: None, columns, .. }) => columns,
            Some(Held { source: Some(s), .. }) => {
                return (0..s.len()).filter(|&r| documents.contains(&super::memo::document_of(s.value(r, at)))).map(|r| (0..width).map(|c| s.value(r, c)).collect()).collect();
            }
            None => return Vec::new(),
        };
        let Some(keys) = columns.get(at) else { return Vec::new() };
        (0..keys.len()).filter(|&r| documents.contains(&super::memo::document_of(keys[r]))).map(|r| columns.iter().map(|col| col[r]).collect()).collect()
    }

    /// Visits held rows without a scratch SQL table or a second column store.
    pub(super) fn visit(&self,rel:&str,width:usize,mut row:impl FnMut(&[i64])) {
        let Some(held)=self.rels.get(rel) else {return;};
        let mut cells=vec![0;width];
        if let Some(source)=&held.source {
            for r in 0..source.len() {
                for (c,v) in cells.iter_mut().enumerate() {*v=source.value(r,c);}
                row(&cells);
            }
        } else {
            for r in 0..held.len() {
                for (c,v) in cells.iter_mut().enumerate() {*v=held.columns[c][r];}
                row(&cells);
            }
        }
    }

    /// Export `rel` as a source of columns `c0…`: its rows, or none when no chunk added any. Made once. The rows
    /// move into it, and the set that kept them distinct is dropped.
    pub(super) fn source(&mut self, rel: &str, width: usize) -> Arc<dyn Source> {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        if let Some(s) = &held.source {
            return Arc::clone(s);
        }
        let columns = std::mem::take(&mut held.columns);
        held.set = HashTable::new();
        held.unset.clear();
        held.unset_docs.clear();
        let names = (0..width).map(|c| format!("c{c}")).collect();
        let values = if columns.is_empty() { vec![Some(Vec::new()); width] } else { columns.into_iter().map(Some).collect() };
        let source = crate::code_rule_dsl::logic::native::component::columns_source(names, values);
        held.source = Some(Arc::clone(&source));
        source
    }
}

#[cfg(test)]
mod tests {
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
}
