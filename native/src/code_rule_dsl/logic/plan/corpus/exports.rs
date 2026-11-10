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

impl Exports {
    /// Export `rel`'s rows are `source` (a session's counted exports, `Counted`): what a chunk added this run is
    /// dropped, as the counted rows hold it.
    pub(super) fn install(&mut self, rel: &str, width: usize, source: Arc<dyn Source>) {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        held.columns = Vec::new();
        held.set = HashTable::new();
        held.source = Some(source);
    }
}

/// Every export's rows with how many files hold each (decision 0096 step 3): kept by a session from run to run, a
/// file's rows counted in when its content is new and out when it changes or leaves, so a run's exports are the
/// rows counted, not every file's rows added again. A row is held while one file holds it.
#[derive(Default)]
pub(super) struct Counted {
    rels: BTreeMap<String, CountedRel>,
}

struct CountedRel {
    columns: Vec<Vec<i64>>,
    counts: Vec<u32>,
    set: HashTable<u32>,
}

impl Counted {
    /// A file's rows of `rel` (`flat` row-major, `width` a row) in (`sign` 1) or out (−1). False when a row counted
    /// out is not held: the counts no longer say what files hold.
    pub(super) fn patch(&mut self, rel: &str, width: usize, flat: &[i64], sign: i64) -> bool {
        if width == 0 || flat.is_empty() {
            return true;
        }
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| CountedRel { columns: vec![Vec::new(); width], counts: Vec::new(), set: HashTable::new() });
        if held.columns.len() != width {
            return false;
        }
        for row in flat.chunks_exact(width) {
            let h = Held::hash(row.iter().copied());
            let columns = &held.columns;
            let same = |&id: &u32| row.iter().enumerate().all(|(c, v)| columns[c][id as usize] == *v);
            match (held.set.find(h, same).copied(), sign > 0) {
                (Some(id), true) => held.counts[id as usize] += 1,
                (None, true) => {
                    let id = held.counts.len() as u32;
                    for (c, v) in row.iter().enumerate() {
                        held.columns[c].push(*v);
                    }
                    held.counts.push(1);
                    let columns = &held.columns;
                    held.set.insert_unique(h, id, |&i| Held::hash(columns.iter().map(|col| col[i as usize])));
                }
                (Some(id), false) if held.counts[id as usize] > 1 => held.counts[id as usize] -= 1,
                (Some(id), false) => held.forget(id, h),
                (None, false) => return false,
            }
        }
        true
    }

    /// Export `rel`'s rows held, as a source of columns `c0…` (a copy: the counts stay for the next run).
    pub(super) fn source(&self, rel: &str, width: usize) -> Arc<dyn Source> {
        let names = (0..width).map(|c| format!("c{c}")).collect();
        let values = match self.rels.get(rel) {
            Some(held) if held.columns.len() == width => held.columns.iter().cloned().map(Some).collect(),
            _ => vec![Some(Vec::new()); width],
        };
        crate::code_rule_dsl::logic::native::component::columns_source(names, values)
    }

    /// `source`, less the rows `minus` counts: a row held by more files than `minus` says.
    pub(super) fn source_without(&self, rel: &str, width: usize, minus: &Counted) -> Arc<dyn Source> {
        let (Some(held), Some(less)) = (self.rels.get(rel), minus.rels.get(rel)) else { return self.source(rel, width) };
        let mut values: Vec<Vec<i64>> = vec![Vec::new(); width];
        if held.columns.len() == width {
            for id in 0..held.counts.len() {
                let row = held.columns.iter().map(|col| col[id]);
                let h = Held::hash(row.clone());
                let gone = less.set.find(h, |&i| held.columns.iter().zip(&less.columns).all(|(a, b)| a[id] == b[i as usize])).map_or(0, |&i| less.counts[i as usize]);
                if held.counts[id] > gone {
                    for (c, v) in row.enumerate() {
                        values[c].push(v);
                    }
                }
            }
        }
        let names = (0..width).map(|c| format!("c{c}")).collect();
        crate::code_rule_dsl::logic::native::component::columns_source(names, values.into_iter().map(Some).collect())
    }

    /// Export `rel`'s rows as columns, each row as many times as files hold it (a base relation's rows, which are
    /// not kept once as an export's are).
    pub(super) fn columns_each(&self, rel: &str, width: usize) -> Vec<Vec<i64>> {
        let mut out = vec![Vec::new(); width];
        if let Some(held) = self.rels.get(rel).filter(|h| h.columns.len() == width) {
            // Mostly each row once: the columns as they are.
            if held.counts.iter().all(|&n| n == 1) {
                return held.columns.clone();
            }
            for (id, &n) in held.counts.iter().enumerate() {
                for _ in 0..n {
                    for (c, col) in out.iter_mut().enumerate() {
                        col.push(held.columns[c][id]);
                    }
                }
            }
        }
        out
    }

    /// Rows held, over every export.
    pub(super) fn rows(&self) -> usize {
        self.rels.values().map(|r| r.counts.len()).sum()
    }
}

impl CountedRel {
    /// Row `id` (hashed `h`) no longer held: the last row moves into its place.
    fn forget(&mut self, id: u32, h: u64) {
        let last = (self.counts.len() - 1) as u32;
        if let Ok(entry) = self.set.find_entry(h, |&i| i == id) {
            entry.remove();
        }
        if id != last {
            let moved = Held::hash(self.columns.iter().map(|col| col[last as usize]));
            if let Ok(mut entry) = self.set.find_entry(moved, |&i| i == last) {
                *entry.get_mut() = id;
            }
            for col in &mut self.columns {
                col[id as usize] = col[last as usize];
            }
            self.counts[id as usize] = self.counts[last as usize];
        }
        for col in &mut self.columns {
            col.pop();
        }
        self.counts.pop();
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

    /// Counted exports hold a row while one file holds it: a row two files hold stays when one counts it out, goes
    /// when both have, and the rows held are those of the files counted in, whatever order they came and went in.
    #[test]
    fn counted_exports_hold_the_rows_of_the_files_counted_in() {
        let mut c = Counted::default();
        let (a, b) = ([1i64, 10, 2, 20, 3, 30], [2i64, 20, 4, 40]);
        assert!(c.patch("r", 2, &a, 1) && c.patch("r", 2, &b, 1));
        assert_eq!(c.rows(), 4);
        assert!(c.patch("r", 2, &a, -1));
        let rows = |c: &Counted| {
            let s = c.source("r", 2);
            let mut v: Vec<(i64, i64)> = (0..s.len()).map(|r| (s.value(r, 0), s.value(r, 1))).collect();
            v.sort();
            v
        };
        assert_eq!(rows(&c), vec![(2, 20), (4, 40)]);
        assert!(c.patch("r", 2, &[5, 50, 2, 20], 1) && c.patch("r", 2, &b, -1));
        assert_eq!(rows(&c), vec![(2, 20), (5, 50)]);
        assert!(!c.patch("r", 2, &[9, 90], -1), "a row not held cannot be counted out");
        // Every row found again after the moves.
        assert!(c.patch("r", 2, &[5, 50, 2, 20], -1));
        assert_eq!(c.rows(), 0);
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
