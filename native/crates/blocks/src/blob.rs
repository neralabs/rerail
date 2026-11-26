//! Blobs: immutable bytes named by their content (P§12 item 1; P§11.4: "source text, tapes: blobs, by content").
//!
//! A blob is never versioned: changed bytes are a new id. Its bytes are read where they lie, in a block: a residency
//! slab's sealed room, a frozen block, or a store's segment loaded to its exact bytes. Here are the names P§12 gives
//! the blocks crate: a block's id, a blob's id, where bytes lie, and the `Blobs` a home of them answers.

/// A block's id: a directory's handle on it, the same on every device, and a kept block's file name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A blob's name: its bytes' sha256.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlobId(pub [u8; 32]);

impl BlobId {
    /// The name of `bytes`.
    pub fn of(bytes: &[u8]) -> BlobId {
        use sha2::Digest as _;
        BlobId(sha2::Sha256::digest(bytes).into())
    }

    /// Its 32 bytes as four words: how a row names it (P§11: "a relation refers to a blob by its id").
    pub fn words(&self) -> [i64; 4] {
        std::array::from_fn(|i| i64::from_le_bytes(self.0[8 * i..8 * i + 8].try_into().expect("eight bytes")))
    }

    /// The blob a row's four words name.
    pub fn of_words(w: &[i64]) -> BlobId {
        let mut b = [0u8; 32];
        for (i, x) in w.iter().take(4).enumerate() {
            b[8 * i..8 * i + 8].copy_from_slice(&x.to_le_bytes()); // copy: not block data (a name's bytes)
        }
        BlobId(b)
    }
}

/// Where bytes lie: a block, an offset in it and a length (P§12's `Placed`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub block: BlockId,
    pub at: usize,
    pub len: usize,
}

/// A home of blobs (P§12's `Blobs`): `get` finds where a blob's bytes lie, never copied; `house` takes bytes where
/// they lie and names them. A home reads through its blocks' directory (`Dir`), and writes its rows by operations a
/// registry holds (`Reg`).
pub trait Blobs {
    type Dir;
    type Reg;

    /// Where blob `id`'s bytes lie, if this home holds it.
    fn get(&self, dir: &Self::Dir, id: &BlobId) -> Result<Option<Placed>, String>;

    /// Houses the bytes at `bytes` where they lie, named by their sha256: the name, and whether they are new to this
    /// home (a second of the same bytes is the first, and its place is not kept).
    fn house(&mut self, reg: &Self::Reg, dir: &mut Self::Dir, bytes: Placed) -> Result<(BlobId, bool), String>;
}

/// The keys a document takes: `1..2^30` (decision 0052 §3; guests, decision 0055, take `2^30..2^31`, so every key's
/// `key << 32` stays positive).
const KEY_SPAN: u64 = (1 << 30) - 1;

/// A document's first key, from its path's hash (decision 0052 §3): a key that means the same in every run, as a
/// position in a file list did not.
pub fn doc_key(path: &str) -> i64 {
    use sha2::Digest as _;
    let hash: [u8; 32] = sha2::Sha256::digest(path.as_bytes()).into();
    (u64::from_le_bytes(hash[..8].try_into().expect("8 bytes")) % KEY_SPAN + 1) as i64
}

/// The key after `key` (a collision's next free one), wrapping within the documents' keys.
pub fn next_key(key: i64) -> i64 {
    ((key as u64) % KEY_SPAN + 1) as i64
}
