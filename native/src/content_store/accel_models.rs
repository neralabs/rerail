//! Compiled Neural Engine models as facts of the store (decision 0098 §3, t33 step 6): `accel_model(key, archive)`,
//! a kept relation of the store's models' space, one partition a model key; the archive is a source of the store
//! (an object, by its SHA-256), pinned with no end, as the table it replaced kept it.
//!
//! The lane (`rerail_accel::ane`) compiles a model once for a fixed relation and keeps the compiled archive here, so
//! a later run, in this process or another, loads it instead of compiling (about 0.4–6 s a model). A store opened on
//! another file replaces this one as the process's (`set_store`). Each call opens the store and closes it after:
//! models are read and written a few times a run, and a run leaves no handle of its store open.

use super::kept_facts::{KeptPut, KeptWhat};
use super::{ContentStore, HashKey};
use rerail_accel::ane::ModelStore;
use rerail_blocks::kept::{FactKey, KeptRead as _};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MODEL: &str = "accel_model";
/// The model key's four words, then the archive's SHA-256's.
const TYPES: &[u8] = b"iiiiiiii";
/// What pins an archive's source.
const OWNER: &str = "rerail.accel-models";

fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.accel-models-space/v1").into()
}

fn ident() -> [u8; 32] {
    Sha256::digest(b"rerail.accel-models/v1\0accel_model").into()
}

fn words(d: &[u8; 32]) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().expect("8 bytes")))
}

fn digest_of(w: &[i64]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, v) in w.iter().take(4).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// A model key's partition.
fn part(key: &[u8; 32]) -> i64 {
    words(key)[0] & i64::MAX
}

pub(super) struct StoreModels {
    path: PathBuf,
}

impl StoreModels {
    pub(super) fn new(path: &Path) -> Self {
        StoreModels { path: path.to_path_buf() }
    }

    fn store(&self) -> Option<ContentStore> {
        ContentStore::open_images(&self.path).ok()
    }
}

impl ModelStore for StoreModels {
    fn get(&self, key: &[u8; 32]) -> Option<Arc<[u8]>> {
        let store = self.store()?;
        let kept = store.kept_space(&space()).ok()??;
        let chain = kept.view();
        let entry = chain.get(&FactKey { rel: MODEL, ident: &ident(), part: part(key) })?;
        let row = (0..entry.rows.len()).map(|i| entry.row(i)).find(|r| digest_of(&r[0..4]) == *key)?;
        let archive = digest_of(&row[4..8]);
        let source = store.source_by_hash(HashKey::from_sha256_digest(archive)).ok()??;
        store.get_source(source).ok().map(Into::into)
    }

    fn put(&self, key: [u8; 32], archive: Arc<[u8]>) {
        // A store that cannot take it leaves the model in this process only: the next run compiles it again.
        let Some(store) = self.store() else { return };
        let Ok(source) = store.put_source(&archive[..], &[]) else { return };
        if store.pin_source(source, OWNER, i64::MAX).is_err() {
            return;
        }
        let digest: [u8; 32] = Sha256::digest(&archive[..]).into();
        // The partition's other models, as kept, with this one in place of any it replaces.
        let mut flat: Vec<i64> = Vec::new();
        if let Ok(Some(kept)) = store.kept_space(&space()) {
            let chain = kept.view();
            if let Some(entry) = chain.get(&FactKey { rel: MODEL, ident: &ident(), part: part(&key) }) {
                for row in (0..entry.rows.len()).map(|i| entry.row(i)).filter(|r| digest_of(&r[0..4]) != key) {
                    flat.extend(row);
                }
            }
        }
        flat.extend(words(&key));
        flat.extend(words(&digest));
        let n = flat.len() / TYPES.len();
        let mut h = Sha256::new();
        for v in &flat {
            h.update(v.to_le_bytes());
        }
        let digest = u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"));
        let put = KeptPut { rel: MODEL.to_owned(), ident: ident(), part: part(&key), digest, reads: Vec::new(), types: TYPES.to_vec(), what: KeptWhat::Picked(Box::new(crate::content_store::kept_facts::BlockFlat::of_rows(&flat, TYPES.len(), true))) };
        store.queue_kept_as(space(), vec![put], "the Neural Engine's model");
        store.settle_kept(&space());
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
        // A second model of the same partition keeps the first; one compiled again replaces its archive.
        let mut other = key;
        other[31] = 8;
        a.put(other, Arc::from(&b"RRMLMC01 other"[..]));
        a.put(key, Arc::from(&b"RRMLMC01 again"[..]));
        assert_eq!(b.get(&other).as_deref(), Some(&b"RRMLMC01 other"[..]));
        assert_eq!(b.get(&key).as_deref(), Some(&b"RRMLMC01 again"[..]));
        // No table keeps them.
        let c = rusqlite::Connection::open(&path).unwrap();
        let tables: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'accel_models'", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 0);
    }
}
