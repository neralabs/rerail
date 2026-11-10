//! The store a run's caches keep their rows in (decision 0103): `content_store::cache_rows`, by key.

use super::super::{CacheStore, Error, Result};
use crate::content_store::{CachePut, ContentStore};
use std::collections::HashMap;

/// The run's content store, as its caches read and write it.
pub(super) struct Store<'s>(pub(super) &'s ContentStore);

impl CacheStore for Store<'_> {
    fn hits(&mut self, id: &[u8; 32], width: usize, keys: &[i64]) -> Result<Vec<(usize, Vec<i64>)>> {
        self.0.cache_hits(id, width, keys).map_err(|e| Error::Plan(format!("a cache's rows: {e}")))
    }

    fn keep(&mut self, id: &[u8; 32], key_width: usize, keys: &[i64], cols: &[usize], width: usize, rows: &[i64]) {
        if key_width == 0 {
            return;
        }
        let mut by_key: HashMap<Vec<i64>, Vec<i64>> = HashMap::new();
        if width > 0 {
            for row in rows.chunks_exact(width) {
                let key: Vec<i64> = cols.iter().map(|&c| row[c]).collect();
                by_key.entry(key).or_default().extend_from_slice(row);
            }
        }
        let puts = keys
            .chunks_exact(key_width)
            .map(|k| CachePut { key: k.to_vec(), width, rows: by_key.remove(k).unwrap_or_default() })
            .collect();
        self.0.queue_cache_rows(*id, puts);
    }
}
