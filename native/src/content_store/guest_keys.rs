//! A guest's key, the store's: one per identity (decision 0323).
//!
//! A guest document (an annotation, an embedded program, a parsed value) has a key hashed from what it is, in its
//! range. Two guests can hash alike; the second takes the next key free. "Free" is the store's: `guest_keys` holds
//! every guest's key, read once a process before the first wave (decision 0104) and held with the store's atoms, so
//! every chunk, run and handle of the process sees each claim at once. A run's new claims are an item of its record
//! (decision 0306). A claim another process wrote first loses: its host's memo entries go, so its next run makes the
//! guest again under the store's key (0052 §3: a key a collision moved only misses).

use super::connection::Cached as _;
use super::record::{codec, Apply};
use super::{ContentStore, StoreResult};
use parking_lot::Mutex;
use rusqlite::{params, OptionalExtension as _};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A guest's identity: a SHA-256 of its range and the parts its key was hashed from.
pub type Identity = [u8; 32];

/// One claim: a guest's identity, its key and its host file (a value's first asker).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub identity: Identity,
    pub key: u32,
    pub path: String,
}

/// The process's guest keys of one store (or of a run without one): every key held, and the claims not yet written.
#[derive(Default)]
pub struct GuestKeys {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The store's rows are in (`ContentStore::guest_keys`).
    read: bool,
    by_identity: HashMap<Identity, u32>,
    taken: HashSet<u32>,
    new: Vec<Claim>,
}

impl GuestKeys {
    /// A run's registry without a store: its chunks share it.
    pub fn in_memory() -> Arc<Self> {
        Arc::default()
    }

    /// The key of the guest `identity`, whose hash is `first` in the range of `span` keys from `base`: the one it
    /// holds, else `first` or the next past every key held, claimed now.
    pub fn key(&self, identity: Identity, base: u32, span: u32, first: u32, path: &str) -> u32 {
        let mut s = self.state.lock();
        if let Some(&key) = s.by_identity.get(&identity) {
            return key;
        }
        let mut k = first % span;
        while !s.taken.insert(base + k) {
            k = (k + 1) % span;
        }
        let key = base + k;
        s.by_identity.insert(identity, key);
        s.new.push(Claim { identity, key, path: path.to_owned() });
        key
    }

    /// The claims not yet written, taken.
    pub fn take_new(&self) -> Vec<Claim> {
        std::mem::take(&mut self.state.lock().new)
    }

    fn hold(s: &mut State, identity: Identity, key: u32) {
        s.by_identity.insert(identity, key);
        s.taken.insert(key);
    }
}

impl ContentStore {
    /// The store's guest keys, as this process holds them: read once a process, before the run's first wave (decision
    /// 0104); the waves find and claim keys in memory.
    pub fn guest_keys(&self) -> StoreResult<Arc<GuestKeys>> {
        let keys = self.atoms.guest_keys();
        self.read_guest_keys(&keys)?;
        Ok(keys)
    }

    /// Reads the store's rows into `keys`, unless it holds them.
    fn read_guest_keys(&self, keys: &GuestKeys) -> StoreResult<()> {
        let mut s = keys.state.lock();
        if s.read {
            return Ok(());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached("SELECT identity, key FROM guest_keys")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (identity, key) = row?;
            if let (Ok(identity), Ok(key)) = (Identity::try_from(identity.as_slice()), u32::try_from(key)) {
                GuestKeys::hold(&mut s, identity, key);
            }
        }
        s.read = true;
        Ok(())
    }

    /// Writes this process's new claims behind the run, in its record (decision 0306).
    pub fn record_guest_keys(&self) {
        let claims = self.atoms.guest_keys().take_new();
        if claims.is_empty() {
            return;
        }
        fn apply(cx: &Apply<'_>, body: codec::Body) -> StoreResult<()> {
            let claims: Vec<Claim> = codec::from_body("the guests' keys", body)?;
            cx.store.put_guest_keys(&claims).map(drop)
        }
        self.record("the guests' keys", Vec::new(), &claims, None, apply);
    }

    /// Inserts `claims`. A claim whose identity or key another process wrote first is not: its host's memo entries
    /// are dropped, and this process holds the store's rows in its place. The hosts whose entries were dropped.
    pub(crate) fn put_guest_keys(&self, claims: &[Claim]) -> StoreResult<Vec<String>> {
        let lost: Vec<(Claim, Vec<(Identity, u32)>)> = self.immediate_transaction(|tx| {
            let mut lost = Vec::new();
            for c in claims {
                tx.execute_cached("INSERT OR IGNORE INTO guest_keys(identity, key, path) VALUES (?1, ?2, ?3)", params![&c.identity[..], i64::from(c.key), c.path])?;
                let held: Option<i64> = tx.query_row_cached("SELECT key FROM guest_keys WHERE identity = ?1", [&c.identity[..]], |r| r.get(0)).optional()?;
                let holder: Option<Vec<u8>> = tx.query_row_cached("SELECT identity FROM guest_keys WHERE key = ?1", [i64::from(c.key)], |r| r.get(0)).optional()?;
                if held == Some(i64::from(c.key)) {
                    continue;
                }
                tx.execute_cached("DELETE FROM memo_entries WHERE path = ?1", [&c.path])?;
                let mut stored = Vec::new();
                if let Some(key) = held.and_then(|k| u32::try_from(k).ok()) {
                    stored.push((c.identity, key));
                }
                if let Some(identity) = holder.and_then(|i| Identity::try_from(i.as_slice()).ok()) {
                    stored.push((identity, c.key));
                }
                lost.push((c.clone(), stored));
            }
            Ok(lost)
        })?;
        if lost.is_empty() {
            return Ok(Vec::new());
        }
        let keys = self.atoms.guest_keys();
        let mut s = keys.state.lock();
        for (c, stored) in &lost {
            if s.by_identity.get(&c.identity) == Some(&c.key) {
                s.by_identity.remove(&c.identity);
            }
            for &(identity, key) in stored {
                GuestKeys::hold(&mut s, identity, key);
            }
        }
        Ok(lost.into_iter().map(|(c, _)| c.path).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Identity {
        [n; 32]
    }

    /// Another process's view of `store`: its registry, read from the table.
    fn another_process(store: &ContentStore) -> GuestKeys {
        let keys = GuestKeys::default();
        store.read_guest_keys(&keys).unwrap();
        keys
    }

    /// A guest's key is the store's: a second identity hashing alike probes past it, and another process reading the
    /// store gives each identity the key it was given, and a third the next free.
    #[test]
    fn a_guests_key_is_its_identitys_in_every_process_and_a_collision_probes_past_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let keys = store.guest_keys().unwrap();
        let (base, span) = (1000, 16);
        assert_eq!(keys.key(id(1), base, span, 5, "a.py"), 1005);
        assert_eq!(keys.key(id(2), base, span, 5, "b.py"), 1006, "a collision probes past the key held");
        assert_eq!(keys.key(id(1), base, span, 9, "a.py"), 1005, "an identity keeps its key");
        assert_eq!(keys.key(id(3), base, span, 15, "c.py"), 1015);
        assert_eq!(keys.key(id(4), base, span, 15, "d.py"), 1000, "the probe wraps in its range");
        assert!(store.put_guest_keys(&keys.take_new()).unwrap().is_empty());
        let other = another_process(&store);
        assert_eq!(other.key(id(2), base, span, 5, "b.py"), 1006);
        assert_eq!(other.key(id(5), base, span, 5, "e.py"), 1007, "past every key the store holds");
        assert_eq!(other.take_new(), vec![Claim { identity: id(5), key: 1007, path: "e.py".into() }]);
    }

    /// Two processes claim one key: the second's claim is not written, its host's memo entries go, and its process
    /// holds the store's rows from then on.
    #[test]
    fn a_claim_another_process_wrote_first_drops_its_hosts_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let keys = store.guest_keys().unwrap();
        // This process claims 1005 for identity 1 (host b.py); another wrote 1005 for identity 2 (host a.py) first.
        assert_eq!(keys.key(id(1), 1000, 16, 5, "b.py"), 1005);
        let first = another_process(&store);
        assert_eq!(first.key(id(2), 1000, 16, 5, "a.py"), 1005);
        assert!(store.put_guest_keys(&first.take_new()).unwrap().is_empty());
        {
            let c = store.connection.lock().unwrap();
            for (part, path) in [(1, "a.py"), (2, "b.py")] {
                c.execute("INSERT INTO memo_entries(space, part, path, digest, doc, pack, at, len, raw, framed) VALUES (x'00', ?1, ?2, x'00', 0, 0, 0, 0, 0, 1)", params![part, path]).unwrap();
            }
        }
        assert_eq!(store.put_guest_keys(&keys.take_new()).unwrap(), vec!["b.py".to_owned()]);
        let paths: Vec<String> = store.connection.lock().unwrap().prepare("SELECT path FROM memo_entries").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(paths, ["a.py"], "the losing claim's host misses next run");
        assert_eq!(keys.key(id(2), 1000, 16, 5, "a.py"), 1005, "the store's row is held");
        assert_eq!(keys.key(id(1), 1000, 16, 5, "b.py"), 1006, "the loser takes a free key next run");
    }
}
