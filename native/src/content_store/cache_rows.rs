//! A cache's rows (decision 0103): for each cache (named by a digest of its output rules and its relations'
//! columns, with the engine's build) and each key the cache has seen, the cache's rows for that key. A key with no
//! rows is kept too, as a key seen: its output rule does not run again.
//!
//! This is where a cache's two relations live between runs, as every relation lives in the store: the key relation's
//! rows are the keys, the output's rows are each key's rows. It is read by key (`cache_hits`), so a run reads only
//! the keys it has, and written by the store's one writer (decision 0102), behind the run.

use super::{ContentStore, StoreResult};
use rusqlite::{OptionalExtension as _, params};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS cache_rows(cache BLOB NOT NULL, key BLOB NOT NULL, width INTEGER NOT NULL, \
     rows BLOB NOT NULL, PRIMARY KEY (cache, key)) WITHOUT ROWID;";

fn bytes(values: &[i64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn values(bytes: &[u8]) -> Vec<i64> {
    bytes.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().expect("eight bytes"))).collect()
}

fn cache_key(cache: &[u8; 32]) -> super::write_back::Key {
    super::write_back::Key::of(&[b"cache_rows", cache])
}

/// One key of a cache as written: the key's values, and its rows (flat, `width` values each).
pub struct CachePut {
    pub key: Vec<i64>,
    pub width: usize,
    pub rows: Vec<i64>,
}

impl ContentStore {
    fn cache_rows_table(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let n: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='cache_rows'", [], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The keys of `keys` (flat, `width` values each) that cache `cache` has seen: each one's index in `keys`, and
    /// its rows (flat).
    pub fn cache_hits(&self, cache: &[u8; 32], width: usize, keys: &[i64]) -> StoreResult<Vec<(usize, Vec<i64>)>> {
        self.settle(&cache_key(cache));
        if !self.cache_rows_table()? {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT rows FROM cache_rows WHERE cache = ?1 AND key = ?2")?;
        let mut out = Vec::new();
        let n = if width == 0 { usize::from(!keys.is_empty()) } else { keys.len() / width };
        for i in 0..n {
            let key = bytes(&keys[i * width..i * width + width]);
            if let Some(rows) = st.query_row(params![&cache[..], key], |r| r.get::<_, Vec<u8>>(0)).optional()? {
                out.push((i, values(&rows)));
            }
        }
        Ok(out)
    }

    /// Keeps `puts` for cache `cache`, each replacing its key's rows.
    pub fn keep_cache_rows(&self, cache: &[u8; 32], puts: &[CachePut]) -> StoreResult<()> {
        self.immediate_transaction_once(|t| {
            t.execute_batch(SCHEMA)?;
            let mut ins = t.prepare_cached(
                "INSERT INTO cache_rows(cache, key, width, rows) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(cache, key) DO UPDATE SET width = excluded.width, rows = excluded.rows",
            )?;
            for p in puts {
                ins.execute(params![&cache[..], bytes(&p.key), p.width as i64, bytes(&p.rows)])?;
            }
            Ok(())
        })
    }

    /// `keep_cache_rows`, queued behind the run (decision 0102): written by the store's writer. A read of the cache
    /// waits for it (`cache_hits`).
    pub fn queue_cache_rows(&self, cache: [u8; 32], puts: Vec<CachePut>) {
        if puts.is_empty() {
            return;
        }
        let bytes = puts.iter().map(|p| 8 * (p.key.len() + p.rows.len())).sum();
        self.write_back("a cache's rows", bytes, vec![(cache_key(&cache), None)], Box::new(move |store| {
            // Attempted once: a busy store leaves these keys unkept, and the next run makes them again.
            let _ = store.keep_cache_rows(&cache, &puts);
            Ok(())
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caches_rows_are_read_by_key() {
        let dir = std::env::temp_dir().join(format!("rerail-cache-rows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let cache = [3u8; 32];
        assert!(store.cache_hits(&cache, 1, &[1, 2]).unwrap().is_empty());
        store.keep_cache_rows(&cache, &[CachePut { key: vec![1], width: 2, rows: vec![1, 10, 1, 11] }, CachePut { key: vec![5], width: 2, rows: vec![] }]).unwrap();
        let hits = store.cache_hits(&cache, 1, &[2, 5, 1]).unwrap();
        assert_eq!(hits, vec![(1, vec![]), (2, vec![1, 10, 1, 11])]);
        assert!(store.cache_hits(&[4u8; 32], 1, &[1]).unwrap().is_empty(), "another cache's keys are its own");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
