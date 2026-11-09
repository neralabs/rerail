//! Compiled Neural Engine models as facts of the store (decision 0096 §3): `accel_models`, one archive per model key.
//!
//! The lane (`rerail_accel::ane`) compiles a model once for a fixed relation and keeps the compiled archive here, so
//! a later run, in this process or another, loads it instead of compiling (about 0.4–6 s a model). A store opened on
//! another file replaces this one as the process's (`set_store`). Each call opens its own connection: models are
//! read and written a few times a run.

use rerail_accel::ane::ModelStore;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) struct StoreModels {
    path: PathBuf,
}

impl StoreModels {
    pub(super) fn new(path: &Path) -> Self {
        StoreModels { path: path.to_path_buf() }
    }

    fn connection(&self) -> Option<Connection> {
        let c = Connection::open(&self.path).ok()?;
        c.busy_timeout(std::time::Duration::from_secs(5)).ok()?;
        Some(c)
    }
}

impl ModelStore for StoreModels {
    fn get(&self, key: &[u8; 32]) -> Option<Arc<[u8]>> {
        let c = self.connection()?;
        let got: Option<Vec<u8>> = c.query_row("SELECT archive FROM accel_models WHERE model_key = ?1", params![&key[..]], |r| r.get(0)).optional().ok()?;
        got.map(Into::into)
    }

    fn put(&self, key: [u8; 32], archive: Arc<[u8]>) {
        if let Some(c) = self.connection() {
            // A store that cannot take it leaves the model in this process only: the next run compiles it again.
            let _ = c.execute("INSERT OR REPLACE INTO accel_models (model_key, archive) VALUES (?1, ?2)", params![&key[..], &archive[..]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compiled_model_is_a_fact_of_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("content.sqlite");
        drop(crate::content_store::ContentStore::open(&path).unwrap());
        let (a, b) = (StoreModels::new(&path), StoreModels::new(&path));
        let key = [7u8; 32];
        assert!(a.get(&key).is_none());
        a.put(key, Arc::from(&b"RRMLMC01 model"[..]));
        // Another handle on the file (a later run) reads it back.
        assert_eq!(b.get(&key).as_deref(), Some(&b"RRMLMC01 model"[..]));
    }
}
