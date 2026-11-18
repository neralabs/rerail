//! Segments (decision 0295 §1, §3–§4): the store's deferred writes at rest, in files of mapped pages beside it.
//!
//! A segment is a frozen journal block (`rerail_blocks::journal`): entries one after another at 64 bytes, and a table
//! of them (each one's document key, digest and range). The producer writes it (`SegmentWriter`: on its own thread,
//! into a deferrable lease of the pool, each entry written in place by its maker), seals it, and hands it to the
//! store's writer (`SealedSegments`), which writes it back (`Sealed::write_back`), inserts the rows that point at its
//! entries, and once they commit renames it to its number (`SealedSegments::name`). Readers read an entry where it is
//! (`SegmentBytes`), in the producer's mapping or, from a row, in the segment's file mapped (`SegmentMaps`). The memo's
//! entries (`memo_pack`) and the frames (`frames`) are kept this way, each kind in a directory of its own.

use super::{StoreError, StoreResult};
use rerail_blocks::block::{Frozen, kind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An entry's bytes where they are: a range of its segment, mapped (or, for bytes not stored, a block of their own).
#[derive(Clone)]
pub struct SegmentBytes {
    block: Frozen,
    at: usize,
    len: usize,
}

impl SegmentBytes {
    pub fn bytes(&self) -> &[u8] {
        self.block.bytes(self.at, self.at + self.len)
    }
    /// The segment's block, and the range's offset in it.
    pub fn block(&self) -> (&Frozen, usize) {
        (&self.block, self.at)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// `bytes` alone, in a block of their own (a test's entry, an entry not stored).
    pub fn of(bytes: &[u8]) -> SegmentBytes {
        let block = rerail_blocks::alloc::host(kind::JOURNAL, rerail_blocks::journal::reserve_for(1, std::iter::once(bytes.len()), ALIGN).next_multiple_of(rerail_blocks::block::PAGE));
        rerail_blocks::journal::init(&block, 1);
        let at = rerail_blocks::journal::put(&block, bytes, ALIGN, 0, 0);
        SegmentBytes { block: block.freeze(), at, len: bytes.len() }
    }
}

impl std::fmt::Debug for SegmentBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SegmentBytes({} bytes at {})", self.len, self.at)
    }
}

/// An entry's alignment in a segment: 0239's images are read at 16 bytes; 64 keeps each on its own cache lines.
pub(super) const ALIGN: usize = 64;

/// Where an entry is, as a row names it: its segment's number, its offset and length.
#[derive(Clone, Copy)]
pub(super) struct Place {
    pub seg: i64,
    pub at: u64,
    pub len: usize,
}

/// Makes `dir` (a store's segments: facts derived from its sources) private to its owner where it is made, as the
/// store's own files are (`private_files`).
fn private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// The bytes of the store's segments on disk (the memo's, the frames', the tapes' and sources'): the directories
/// beside its file, as the store's disk budget counts them.
pub(super) fn segment_bytes(database: &Path) -> u64 {
    let mut bytes = 0;
    for suffix in [".memo", ".frames", ".objects"] {
        let mut dir = database.as_os_str().to_owned();
        dir.push(suffix);
        let Ok(entries) = std::fs::read_dir(PathBuf::from(dir)) else { continue };
        bytes += entries.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum::<u64>();
    }
    bytes
}

/// The file of segment `seg` in `dir`.
pub(super) fn segment_path(dir: &Path, seg: i64) -> PathBuf {
    dir.join(format!("{seg}.seg"))
}

/// The segments a read finds by rows, each mapped once: one whose file is not there or does not check is `None`.
pub(super) struct SegmentMaps {
    dir: PathBuf,
    open: HashMap<i64, Option<Frozen>>,
}

impl SegmentMaps {
    pub(super) fn new(dir: PathBuf) -> SegmentMaps {
        SegmentMaps { dir, open: HashMap::new() }
    }
    /// The entry at `w`, where it is: `None` when its segment is not there, or holds no such range.
    pub(super) fn at(&mut self, w: Place) -> Option<SegmentBytes> {
        let block = self.open.entry(w.seg).or_insert_with(|| rerail_blocks::journal::open(&segment_path(&self.dir, w.seg)).ok()).as_ref()?;
        let at = w.at as usize;
        (at.checked_add(w.len)? <= block.used()).then(|| SegmentBytes { block: block.clone(), at, len: w.len })
    }
}

/// The entries of a batch, written into segments on the producer's thread (decision 0295 §1: the wave's, into its
/// own leases): each placed in the open lease and written there by its maker (`put`), with no copy of it made
/// first; a lease without room for the next is sealed and another opened. `seal` hands them on to the store's
/// writer, and they are read in place meanwhile (`SealedSegments::entry`).
pub struct SegmentWriter {
    dir: PathBuf,
    /// The entries still to come (each lease's table has room for them all) and the bytes a lease is made of.
    left: usize,
    lease: usize,
    open: Option<Open>,
    done: Vec<Done>,
    placed: Vec<(usize, usize, usize)>,
}

/// A segment being written: a deferrable lease of the pool, or (no mapping, WASI) a block in memory.
enum Open {
    #[cfg(unix)]
    Mapped(rerail_blocks::deferrable::Deferrable),
    #[cfg(not(unix))]
    Memory(rerail_blocks::block::Block, PathBuf),
}

/// A segment sealed, on its way to the writer, then written back; its file under its writer's name until the
/// batch's rows name it.
enum Done {
    #[cfg(unix)]
    Mapped(rerail_blocks::deferrable::Sealed, Frozen),
    #[cfg(not(unix))]
    Memory(Frozen, PathBuf),
    /// Written back: its block, read in place, and its file.
    Written(Frozen, PathBuf),
}

impl Open {
    fn block(&self) -> &rerail_blocks::block::Block {
        match self {
            #[cfg(unix)]
            Open::Mapped(d) => d.block(),
            #[cfg(not(unix))]
            Open::Memory(b, _) => b,
        }
    }
    fn seal(self) -> Done {
        match self {
            #[cfg(unix)]
            Open::Mapped(d) => {
                let sealed = d.seal();
                let reader = sealed.reader();
                Done::Mapped(sealed, reader)
            }
            #[cfg(not(unix))]
            Open::Memory(b, path) => Done::Memory(b.freeze(), path),
        }
    }
}

impl Done {
    fn reader(&self) -> &Frozen {
        match self {
            #[cfg(unix)]
            Done::Mapped(_, r) => r,
            #[cfg(not(unix))]
            Done::Memory(r, _) => r,
            Done::Written(r, _) => r,
        }
    }
    fn file(&self) -> &Path {
        match self {
            #[cfg(unix)]
            Done::Mapped(s, _) => s.path(),
            #[cfg(not(unix))]
            Done::Memory(_, f) => f,
            Done::Written(_, f) => f,
        }
    }
    /// The writer's half: its pages written back and its state frozen (`Sealed::write_back`; a block no device
    /// wrapped stays mapped, its readers reading on), or (no mapping) its bytes written as one file. Once.
    fn write_back(self) -> StoreResult<Done> {
        match self {
            #[cfg(unix)]
            Done::Mapped(sealed, reader) => {
                let path = sealed.path().to_owned();
                let frozen = sealed.write_back()?;
                drop(reader);
                Ok(Done::Written(frozen, path))
            }
            #[cfg(not(unix))]
            Done::Memory(frozen, path) => {
                std::fs::write(&path, frozen.bytes(0, frozen.used()))?;
                Ok(Done::Written(frozen, path))
            }
            written @ Done::Written(..) => Ok(written),
        }
    }
}

impl SegmentWriter {
    /// A writer of `n` entries into segments in `dir`, about `estimate` bytes in all: the size of its first lease.
    pub fn new(dir: &Path, n: usize, estimate: usize) -> SegmentWriter {
        SegmentWriter { dir: dir.to_owned(), left: n, lease: estimate, open: None, done: Vec::new(), placed: Vec::new() }
    }

    /// Places the next entry, `len` bytes of document `key` and content `digest`, and has `write` write it there.
    pub fn put(&mut self, len: usize, key: i64, digest: &[u8; 32], write: impl FnOnce(&mut [u8])) -> StoreResult<()> {
        let digest = u64::from_le_bytes(digest[..8].try_into().expect("eight bytes"));
        for fresh in [false, true] {
            if fresh || self.open.is_none() {
                if let Some(open) = self.open.take() {
                    self.done.push(open.seal());
                }
                let need = rerail_blocks::journal::reserve_for(self.slots(), std::iter::once(len), ALIGN);
                self.open = Some(self.lease(self.lease.max(need))?);
            }
            let open = self.open.as_ref().expect("an open segment");
            if let Some(at) = rerail_blocks::journal::place(open.block(), len, ALIGN, key as u32, digest) {
                // SAFETY: `at..at + len` was just placed in the open block, for this entry alone.
                write(unsafe { std::slice::from_raw_parts_mut(open.block().ptr::<u8>(at), len) });
                self.placed.push((self.done.len(), at, len));
                self.left = self.left.saturating_sub(1);
                return Ok(());
            }
        }
        Err(StoreError::Invalid(format!("a memo entry of {len} bytes found no room in a segment made for it")))
    }

    fn lease(&self, bytes: usize) -> StoreResult<Open> {
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        private_dir(&self.dir)?;
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = self.dir.join(format!("w-{}-{n}.seg", std::process::id()));
        #[cfg(target_os = "macos")]
        let open = Open::Mapped(rerail_accel::gpu::shared::lease_deferrable_host(kind::JOURNAL, bytes, &file).map_err(StoreError::Invalid)?);
        #[cfg(all(unix, not(target_os = "macos")))]
        let open = Open::Mapped(rerail_blocks::deferrable::Deferrable::create(&file, kind::JOURNAL, bytes, true)?);
        #[cfg(not(unix))]
        let open = Open::Memory(rerail_blocks::alloc::host(kind::JOURNAL, bytes.next_multiple_of(rerail_blocks::block::PAGE)), file);
        rerail_blocks::journal::init(open.block(), self.slots());
        Ok(open)
    }

    /// The entries a new segment's table has room for: those still to come, and once more came than were said
    /// (`left` spent), as many as were placed so far. A table of one past the estimate made a file per entry: 13,677
    /// segments for a write of 166 MB on 50 files of B-now, against 34, each leased, mapped, named and written
    /// back (the store writer's frames, 0.09 → 3.3 s of CPU).
    fn slots(&self) -> usize {
        if self.left > 0 { self.left } else { self.placed.len().max(64) }
    }

    /// The entries placed so far.
    pub fn len(&self) -> usize {
        self.placed.len()
    }

    /// Tests: entry `i`'s bytes as written, before the segments are sealed.
    #[cfg(test)]
    pub fn entry_bytes(&self, i: usize) -> Option<Vec<u8>> {
        let &(seg, at, len) = self.placed.get(i)?;
        if seg < self.done.len() {
            return Some(self.done[seg].reader().bytes(at, at + len).to_vec());
        }
        let open = self.open.as_ref()?;
        // SAFETY: `at..at + len` was placed in the open block and written by its `put`.
        Some(unsafe { std::slice::from_raw_parts(open.block().ptr::<u8>(at), len) }.to_vec())
    }

    /// Seals the open segment: the entries go to the writer, read in place meanwhile.
    pub fn seal(mut self) -> SealedSegments {
        if let Some(open) = self.open.take() {
            self.done.push(open.seal());
        }
        SealedSegments { done: self.done.into_iter().map(Some).collect(), placed: self.placed, named: false }
    }
}

/// A batch's sealed segments (`SegmentWriter::seal`) and where each entry is in them, in the order put. Its files
/// are removed when it is dropped, unless its rows name them (`name`).
pub struct SealedSegments {
    done: Vec<Option<Done>>,
    placed: Vec<(usize, usize, usize)>,
    pub(super) named: bool,
}

impl SealedSegments {
    /// Entry `i`, where it is.
    pub fn entry(&self, i: usize) -> SegmentBytes {
        let (seg, at, len) = self.placed[i];
        SegmentBytes { block: self.done[seg].as_ref().expect("a segment").reader().clone(), at, len }
    }
    pub fn len(&self) -> usize {
        self.placed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }
    /// Writes every segment back that is not yet (the writer's half; again after a failure, only those left).
    pub(super) fn write_back(&mut self) -> StoreResult<()> {
        for d in &mut self.done {
            match d.take() {
                Some(segment) => *d = Some(segment.write_back()?),
                None => return Err(StoreError::Invalid("a memo segment whose write-back failed".into())),
            }
        }
        Ok(())
    }
    /// Each segment's file, under its writer's name until `name`.
    pub(super) fn files(&self) -> Vec<PathBuf> {
        self.done.iter().flatten().map(|d| d.file().to_owned()).collect()
    }
}

impl SealedSegments {
    /// Entry `i`'s segment (an index of `files`), offset and length.
    pub(super) fn placed(&self, i: usize) -> (usize, usize, usize) {
        self.placed[i]
    }
    /// Once the rows naming them are committed: each segment renamed to its number (`numbers`, in `files`' order) in
    /// `dir`. A reader that found a row meanwhile misses.
    pub(super) fn name(&mut self, dir: &Path, numbers: &[i64]) -> StoreResult<()> {
        self.named = true;
        for (file, seg) in self.files().iter().zip(numbers) {
            std::fs::rename(file, segment_path(dir, *seg))?;
        }
        Ok(())
    }
}

impl Drop for SealedSegments {
    fn drop(&mut self) {
        if !self.named {
            for f in self.files() {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

/// `entries` (each an image, its document key and content digest) in new segments of `dir`, written back.
pub(super) fn write_segments(dir: &Path, entries: &[(&[u8], i64, &[u8; 32])]) -> StoreResult<SealedSegments> {
    let mut w = SegmentWriter::new(dir, entries.len(), rerail_blocks::journal::reserve_for(entries.len(), entries.iter().map(|e| e.0.len()), ALIGN));
    for (bytes, key, digest) in entries {
        w.put(bytes.len(), *key, digest, |out| out.copy_from_slice(bytes))?;
    }
    let mut segments = w.seal();
    segments.write_back()?;
    Ok(segments)
}

