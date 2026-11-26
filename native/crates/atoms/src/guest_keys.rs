//! The guests' keys' registry (decision 0104): every key this process holds of one store (or of a run without one),
//! and the claims not yet written. The store reads and writes its rows (`ContentStore::guest_keys`).
use parking_lot::Mutex;
use rerail_store_base::content_store::StoreResult;
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

    /// Holds the store's `rows` (identity, key), read once a process (`rows` is called only the first time).
    pub fn read_once(&self, rows: impl FnOnce() -> StoreResult<Vec<(Identity, u32)>>) -> StoreResult<()> {
        let mut s = self.state.lock();
        if s.read {
            return Ok(());
        }
        for (identity, key) in rows()? {
            GuestKeys::hold(&mut s, identity, key);
        }
        s.read = true;
        Ok(())
    }

    /// The claims another process wrote first: each one's own entry dropped where it is still held, and the store's
    /// rows held in its place (`ContentStore::put_guest_keys`).
    pub fn replace_lost(&self, lost: &[(Claim, Vec<(Identity, u32)>)]) {
        let mut s = self.state.lock();
        for (c, stored) in lost {
            if s.by_identity.get(&c.identity) == Some(&c.key) {
                s.by_identity.remove(&c.identity);
            }
            for &(identity, key) in stored {
                GuestKeys::hold(&mut s, identity, key);
            }
        }
    }

    fn hold(s: &mut State, identity: Identity, key: u32) {
        s.by_identity.insert(identity, key);
        s.taken.insert(key);
    }
}
