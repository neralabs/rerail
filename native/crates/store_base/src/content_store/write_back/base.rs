//! The name a queued write goes by (`write_back::Key`), which its makers compute and the store's writer keeps.

/// What a job writes, named so a read of it can find the job (a digest of the table and its key).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key(pub [u8; 32]);

impl Key {
    /// The key of `parts` (a table's name, then its key's bytes).
    pub fn of(parts: &[&[u8]]) -> Key {
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        for p in parts {
            hash.update((p.len() as u64).to_le_bytes());
            hash.update(p);
        }
        Key(hash.finalize().into())
    }
}
