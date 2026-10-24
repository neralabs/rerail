//! Bounded, document-owned metadata. Complete keys compare without hash aliases;
//! rows, statements and SQLite connections never enter this cache.
use super::{Columns, Registry};
use std::collections::{BTreeMap, VecDeque};
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Key {
    query: String,
    inputs: Registry,
    deferred: Vec<String>,
}
impl Key {
    pub(crate) fn new<'a>(
        query: &str,
        schemas: &Registry,
        reads: impl Iterator<Item = &'a String>,
        deferred: impl Iterator<Item = &'a String>,
    ) -> Self {
        Self {
            query: query.to_owned(),
            inputs: reads
                .map(|name| (name.clone(), schemas[name].clone()))
                .collect(),
            deferred: deferred.cloned().collect(),
        }
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.query.len()
            + self
                .inputs
                .iter()
                .map(|(name, columns)| name.len() + column_bytes(columns))
                .sum::<usize>()
            + self
                .deferred
                .iter()
                .map(|name| name.len() + std::mem::size_of::<String>())
                .sum::<usize>()
    }
}
fn column_bytes(columns: &Columns) -> usize {
    columns
        .iter()
        .map(|c| std::mem::size_of_val(c) + c.name.len() + c.declared_type.len())
        .sum()
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: BTreeMap<Key, Columns>,
    order: VecDeque<Key>,
    bytes: usize,
}
impl Cache {
    pub(crate) fn get(&self, key: &Key) -> Option<&Columns> {
        self.entries.get(key)
    }
    pub(crate) fn insert(&mut self, key: Key, columns: Columns) {
        // Keys are owned by both the lookup map and FIFO queue. Charge both copies.
        let bytes = 2 * key.bytes() + column_bytes(&columns);
        if bytes > MAX_BYTES || self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            let oldest = self.order.pop_front().expect("nonempty schema cache");
            let columns = self.entries.remove(&oldest).expect("schema cache key");
            self.bytes -= 2 * oldest.bytes() + column_bytes(&columns);
        }
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.entries.insert(key, columns);
    }
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;
