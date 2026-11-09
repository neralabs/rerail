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
}

impl Held {
    fn new(width: usize) -> Self {
        Held { columns: vec![Vec::new(); width], set: HashTable::new(), source: None }
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

    /// Export `rel` as a source of columns `c0…`: its rows, or none when no chunk added any. Made once. The rows
    /// move into it, and the set that kept them distinct is dropped.
    pub(super) fn source(&mut self, rel: &str, width: usize) -> Arc<dyn Source> {
        let held = self.rels.entry(rel.to_owned()).or_insert_with(|| Held::new(width));
        if let Some(s) = &held.source {
            return Arc::clone(s);
        }
        let columns = std::mem::take(&mut held.columns);
        held.set = HashTable::new();
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
}
