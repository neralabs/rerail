//! The store's records as the crates above it hold them (decision 0401: blocks in mapped files): a tape's or a
//! source's bytes where they lie (`ObjectBytes`, `ObjectFrame`, `TapeFrame`), the programs recorded with a file's tape,
//! the derivations an observer records, and the atoms' ids. Data, which the store makes and reads: a crate that holds
//! them needs no store (`tools/cratify/fanout.py`, crate split step 10b).

use super::{CachedBytes, SegmentBytes, StoreError, StoreResult, StoredDocument};
use std::sync::Arc;

pub type Sym = u32;

/// The digest an unbound name reads as: a derivation that probed a name nobody bound records it,
/// and is invalidated when the name is first bound.
pub const ABSENT: [u8; 32] = [0; 32];

/// A name and the digest an observer saw it bound to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub name: String,
    pub digest: [u8; 32],
}

/// One input of a derivation: a name and the digest it read, or an immutable digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivationInput {
    pub name: Option<String>,
    pub digest: [u8; 32],
}

/// A derivation as run: its output name, its producer (plan identity, revision included), what it
/// read, the digest of what it produced, its status and its cost.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivationRecord {
    pub output: String,
    pub producer: [u8; 32],
    pub inputs: Vec<DerivationInput>,
    pub digest: [u8; 32],
    pub status: i64,
    pub cost: i64,
}

/// A stored derivation, as the frontier names it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Derivation {
    pub output: String,
    pub producer: [u8; 32],
    pub status: i64,
    pub cost: i64,
}

/// A derivation by atoms, as the overlay and the writer hold it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDerivation {
    pub producer: [u8; 32],
    pub status: i64,
    pub cost: i64,
    pub inputs: Vec<(Option<Sym>, [u8; 32])>,
}

/// A tape's bytes where they live, which the writer takes as they are (decision 0353 step 4a: a tape's home, a
/// slab's room).
pub type TapeBytes = Arc<dyn TapeHeld>;

/// What holds a tape's bytes: any byte owner, which its maker can find again (`Any`: the engine's tape home, whose
/// room a later read of the queued tape reads in place).
pub trait TapeHeld: AsRef<[u8]> + std::any::Any + Send + Sync {}
impl<T: AsRef<[u8]> + std::any::Any + Send + Sync> TapeHeld for T {}

/// A stored object's bytes: decompressed into memory (a row of its own), or where they are in a segment, mapped.
#[derive(Clone)]
pub enum ObjectBytes {
    Heap(Arc<Vec<u8>>),
    Mapped(SegmentBytes),
    /// A tape queued for the writer, in its home (decision 0353 step 4a: a slab's room, read there).
    Tape(TapeBytes),
}

impl std::ops::Deref for ObjectBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ObjectBytes::Heap(b) => b,
            ObjectBytes::Mapped(b) => b.bytes(),
            ObjectBytes::Tape(b) => (**b).as_ref(),
        }
    }
}

impl ObjectBytes {
    /// Its bytes as a vector of its own: a decompressed one as it is, a mapped one copied.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            ObjectBytes::Heap(b) => Arc::try_unwrap(b).unwrap_or_else(|b| (*b).clone()),
            ObjectBytes::Mapped(b) => b.bytes().to_vec(),
            ObjectBytes::Tape(b) => (*b).as_ref().to_vec(),
        }
    }
}

impl AsRef<[u8]> for ObjectBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl ObjectBytes {
    /// Whether they are read where a segment holds them, mapped: not decompressed, hashed or copied.
    pub fn in_place(&self) -> bool {
        matches!(self, ObjectBytes::Mapped(_))
    }
}

impl std::fmt::Debug for ObjectBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectBytes({} bytes)", self.len())
    }
}

/// A stored object as its row names it: its bytes as they are, in a segment of `<store>.objects/`, mapped. Every
/// source and tape is kept so (decision 0295 step 5; no row holds an object's bytes since floor 7, 2026-11-19).
#[derive(Clone)]
pub struct ObjectFrame(SegmentBytes);

impl std::fmt::Debug for ObjectFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectFrame({} bytes)", self.len())
    }
}

impl ObjectFrame {
    /// The object a segment's range holds (`object_segments::row_frame`: a row's `seg`, `seg_at`, `seg_len`).
    pub fn of(bytes: SegmentBytes) -> Self {
        Self(bytes)
    }
    /// Its bytes where they are.
    pub fn segment(&self) -> &SegmentBytes {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Its bytes, read in place: refused where they are not `raw_length` long, as its row says.
    pub fn bytes(&self, raw_length: usize) -> std::io::Result<ObjectBytes> {
        if self.0.len() != raw_length {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "an object's length"));
        }
        Ok(ObjectBytes::Mapped(self.0.clone()))
    }
    /// Its first `n` bytes.
    pub fn head(&self, n: usize) -> &[u8] {
        &self.0.bytes()[..n.min(self.0.len())]
    }
    /// Its bytes into `out`, which is their length: a load from the segment into the object's home (a slab's room).
    pub fn copy_into(&self, out: &mut [u8]) -> std::io::Result<()> {
        if self.0.len() != out.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "an object's length"));
        }
        out.copy_from_slice(self.0.bytes());
        Ok(())
    }
}

/// A stored tape as its row has it (`tape_frames_for`): its bytes in a segment.
#[derive(Clone, Debug)]
pub struct TapeFrame {
    raw_length: i64,
    frame: ObjectFrame,
    /// A file's tape: the programs recorded with it (decision 0431 step 6); `None` where not known.
    programs: Option<Arc<[StoredProgram]>>,
}

impl TapeFrame {
    /// A tape of `raw_length` bytes in `frame` (`document_places`: a place its documents' table keeps), its programs
    /// not known there.
    pub fn new(raw_length: i64, frame: ObjectFrame) -> Self {
        Self { raw_length, frame, programs: None }
    }

    /// Records the programs its row names with it (decision 0431 step 6).
    pub fn set_programs(&mut self, programs: Option<Arc<[StoredProgram]>>) {
        self.programs = programs;
    }

    /// A file's tape: the programs recorded with it (decision 0431 step 6); `None` where not known.
    pub fn programs(&self) -> Option<&[StoredProgram]> {
        self.programs.as_deref()
    }

    /// The tape's stored bytes, read in place in its segment.
    pub fn in_place(&self) -> &[u8] {
        self.frame.segment().bytes()
    }

    /// The tape's bytes, read in place: its length checked against its row's.
    pub fn bytes(&self) -> StoreResult<ObjectBytes> {
        self.frame
            .bytes(usize::try_from(self.raw_length).unwrap_or(usize::MAX))
            .map_err(|_| StoreError::Corrupt("tape length does not match metadata".to_owned()))
    }

    /// The frame's size, as stored.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.frame.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frame.is_empty()
    }

    /// The tape's length, as its metadata gives it.
    pub fn raw_len(&self) -> StoreResult<usize> {
        usize::try_from(self.raw_length).map_err(|_| StoreError::Corrupt("a tape's length".to_owned()))
    }

    /// The tape's bytes into `out` (`raw_len` of them), read from its segment (decision 0353 step 4a: a tape's load
    /// writes its one home).
    pub fn copy_into(&self, out: &mut [u8]) -> std::io::Result<()> {
        self.frame.copy_into(out)
    }
}

/// A program recorded with a file's tape (decision 0431 step 6, `TapeFrame::programs`): its row, its text, and its
/// tape's frame.
#[derive(Clone)]
pub struct StoredProgram {
    pub document: StoredDocument,
    pub text: std::sync::Arc<CachedBytes>,
    pub tape: TapeFrame,
}

impl std::fmt::Debug for StoredProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredProgram").field("document", &self.document).field("text", &self.text.len()).finish()
    }
}

/// One path whose object changed between two root trees: a directory (its tree hash) or a file
/// or submodule (its blob or commit hash). `digest` is `ABSENT` where the path no longer exists.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TreeChange {
    pub path: String,
    pub tree: bool,
    pub digest: [u8; 32],
}

/// The changes between two root trees and how many stored trees the walk read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TreeDiff {
    pub changes: Vec<TreeChange>,
    pub trees_read: usize,
}
