//! Segments (decision 0295 §1, §3–§4): the store's deferred writes at rest, in files of mapped pages beside it.
//!
//! A segment is a frozen journal block (`rerail_blocks::journal`): entries one after another at 64 bytes, and a table
//! of them (each one's document key, digest and range). The producer writes it (`SegmentWriter`: on its own thread,
//! into a deferrable lease of the pool, each entry written in place by its maker), seals it, and hands it to the
//! store's writer (`SealedSegments`), which writes it back (`Sealed::write_back`), inserts the rows that point at its
//! entries, and once they commit renames it to its number (`SealedSegments::name`). Readers read an entry where it is
//! (`SegmentBytes`), in the producer's mapping or, from a row, in the segment's file mapped (`SegmentMaps`). The memo's
//! entries (`memo_pack`) and the frames (`frames`) are kept this way, each kind in a directory of its own.

use super::connection::Cached as _;
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

/// The bytes of the store's segments on disk (the memo's, the tapes' and sources', the atoms', the kept facts'): the directories
/// beside its file, as the store's disk budget counts them.
pub(super) fn segment_bytes(database: &Path) -> u64 {
    let mut bytes = 0;
    for suffix in [".memo", ".objects", ".atoms", ".facts"] {
        let mut dir = database.as_os_str().to_owned();
        dir.push(suffix);
        let Ok(entries) = std::fs::read_dir(PathBuf::from(dir)) else { continue };
        // What each file takes on the disk: an open atoms block is sparse, its room not written yet taking none.
        bytes += entries.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| on_disk(&m)).sum::<u64>();
    }
    bytes
}

/// The bytes a file takes on the disk (its allocated blocks; its length where those are not known).
fn on_disk(m: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::blocks(m).saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        m.len()
    }
}

/// The file of segment `seg` in `dir`.
pub(super) fn segment_path(dir: &Path, seg: i64) -> PathBuf {
    dir.join(format!("{seg}.seg"))
}

/// The segments of a kind (`memo_packs`, `object_segments`) the store keeps before compaction merges
/// them, whatever their bytes. A process maps each segment it reads, and the frames a run reads before its first wave
/// are every frame of the store (decision 0104): a process's mappings are about the store's segments, and mappings are
/// a bounded resource (Linux allows 65,530 a process). The bytes bound no count: writes whose entries stay named (a new
/// root's frames, files never parsed again) each left one more segment. Past the budget, the next collection pass that
/// compacts (the passes that compact by bytes) merges all but the oldest half of the budget (`retired`): its cost is
/// the named bytes of those segments, about half the kind's, in one write transaction.
pub(super) const SEGMENT_BUDGET: usize = 1024;

/// A kind's segments as compaction weighs them: their bytes, the bytes their rows name, and their numbers, oldest
/// first.
pub(super) struct SegmentUse {
    pub(super) held: u64,
    pub(super) named: u64,
    pub(super) numbers: Vec<i64>,
}

/// The oldest segment compaction retires, of a kind's `usage`: it and every newer one are merged into one numbered
/// after them all (`merge`), so the order of the kind's entries, which the memo reads from their segments' numbers (its
/// newest first), is kept. Every one where the bytes no row names are at least `floor` and half of the bytes (the space
/// freed pays for the copy); else, past `budget` segments, all but the oldest half of the budget. `None`: no
/// compaction is due.
pub(super) fn retired(usage: &SegmentUse, floor: u64, budget: usize) -> Option<i64> {
    let SegmentUse { held, named, numbers } = usage;
    if *held >= floor && held.saturating_sub(*named) >= held / 2 {
        return numbers.first().copied();
    }
    numbers.get(budget / 2).copied().filter(|_| numbers.len() > budget)
}

/// A kind of segment, as compaction finds and merges it: the table that numbers its segments and the column, and the
/// directory of their files.
pub(super) struct SegmentKind<'a> {
    pub(super) table: &'static str,
    pub(super) column: &'static str,
    pub(super) dir: &'a Path,
}

impl SegmentKind<'_> {
    /// Its segments' numbers, oldest first.
    pub(super) fn numbers(&self, t: &rusqlite::Connection) -> rusqlite::Result<Vec<i64>> {
        let sql = format!("SELECT {c} FROM {t} ORDER BY {c}", c = self.column, t = self.table);
        t.prepare(&sql)?.query_map([], |r| r.get(0))?.collect()
    }

    /// The bytes of the segments `numbers` on disk.
    pub(super) fn held(&self, numbers: &[i64]) -> u64 {
        numbers.iter().map(|&n| std::fs::metadata(segment_path(self.dir, n)).map_or(0, |m| m.len())).sum()
    }

    /// Removes the retired segments' files, once the transaction that retired them committed.
    pub(super) fn remove(&self, retired: &[i64]) {
        for &n in retired {
            let _ = std::fs::remove_file(segment_path(self.dir, n));
        }
    }

    /// Its use, read in `t`: `named` sums the bytes its rows name.
    pub(super) fn usage(&self, t: &rusqlite::Connection, named: &str) -> rusqlite::Result<SegmentUse> {
        let numbers = self.numbers(t)?;
        let named: i64 = t.query_row_cached(named, [], |r| r.get(0))?;
        Ok(SegmentUse { held: self.held(&numbers), named: named as u64, numbers })
    }

    /// Compacts the kind in its write transaction `t` where `usage`, read in `t` (so nothing is written between the
    /// choice and the merge), makes it due: its segments from the oldest `retired` names merged (`merge`), each row
    /// `placed` finds in them moved or lost. A segment a row names is named in place by the time a compaction can read
    /// the row (`SealedSegments::name`), so an entry found missing is lost. Returns the retired segments, whose files
    /// go once `t` commits (`remove`).
    pub(super) fn compact<K>(
        &self,
        t: &rusqlite::Connection,
        usage: &SegmentUse,
        (floor, budget): (u64, usize),
        placed: impl FnOnce(&rusqlite::Connection, i64) -> rusqlite::Result<Vec<Moved<K>>>,
        lost: impl FnMut(&K) -> rusqlite::Result<()>,
        moved: impl FnMut(&K, i64, i64) -> rusqlite::Result<()>,
    ) -> StoreResult<Vec<i64>> {
        let Some(from) = retired(usage, floor, budget) else { return Ok(Vec::new()) };
        merge(t, self, from, placed(t, from)?, lost, moved)
    }
}

/// An entry compaction copies: what names it (`K`, its row), its document key and content digest, and where it is.
pub(super) struct Moved<K> {
    pub(super) key: K,
    pub(super) doc: i64,
    pub(super) digest: [u8; 32],
    pub(super) place: Place,
}

/// Merges `kind`'s segments numbered `from` and after in its write transaction `t` (decision 0295 §4): each of
/// `entries` (theirs, in their order) is copied, in that order, into one new segment numbered after the kind's last,
/// and `moved` points its row there (the segment and the offset); one whose bytes are not there is `lost`. Returns the
/// retired segments.
fn merge<K>(
    t: &rusqlite::Connection,
    kind: &SegmentKind<'_>,
    from: i64,
    entries: Vec<Moved<K>>,
    lost: impl FnMut(&K) -> rusqlite::Result<()>,
    moved: impl FnMut(&K, i64, i64) -> rusqlite::Result<()>,
) -> StoreResult<Vec<i64>> {
    let numbers = kind.numbers(t)?;
    let kept = mapped(kind.dir, entries, lost)?;
    t.execute(&format!("DELETE FROM {} WHERE {} >= ?1", kind.table, kind.column), [from])?;
    if !kept.is_empty() {
        install(t, kind, numbers.last().copied().unwrap_or(0) + 1, &kept, moved)?;
    }
    Ok(numbers.into_iter().filter(|&n| n >= from).collect())
}

/// Each entry's bytes, where they are: one whose segment is not there, or holds no such range, is `lost`.
fn mapped<K>(dir: &Path, entries: Vec<Moved<K>>, mut lost: impl FnMut(&K) -> rusqlite::Result<()>) -> rusqlite::Result<Vec<(Moved<K>, SegmentBytes)>> {
    let mut maps = SegmentMaps::new(dir.to_owned());
    let mut kept = Vec::with_capacity(entries.len());
    for m in entries {
        match maps.at(m.place) {
            Some(bytes) => kept.push((m, bytes)),
            None => lost(&m.key)?,
        }
    }
    Ok(kept)
}

/// `kept`'s bytes written as segment `seg` of `kind`, each row pointed there (`moved`), and the segment named. Bytes
/// several rows name (one place) are written once, each row pointed at the copy.
fn install<K>(t: &rusqlite::Connection, kind: &SegmentKind<'_>, seg: i64, kept: &[(Moved<K>, SegmentBytes)], mut moved: impl FnMut(&K, i64, i64) -> rusqlite::Result<()>) -> StoreResult<()> {
    let mut entries: Vec<(&[u8], i64, &[u8; 32])> = Vec::with_capacity(kept.len());
    let mut entry_of: HashMap<(i64, u64, usize), usize> = HashMap::new();
    let entry: Vec<usize> = kept
        .iter()
        .map(|(m, bytes)| {
            *entry_of.entry((m.place.seg, m.place.at, m.place.len)).or_insert_with(|| {
                entries.push((bytes.bytes(), m.doc, &m.digest));
                entries.len() - 1
            })
        })
        .collect();
    let mut placed = write_segments(kind.dir, &entries, crate::content_store::connection::flush_of(t))?;
    if placed.files().len() != 1 {
        return Err(StoreError::Invalid(format!("the compaction of {} made more than one segment", kind.table)));
    }
    t.execute(&format!("INSERT INTO {}({}) VALUES (?1)", kind.table, kind.column), [seg])?;
    for ((m, _), &i) in kept.iter().zip(&entry) {
        moved(&m.key, seg, placed.placed(i).1 as i64)?;
    }
    placed.name(kind.dir, &[seg])
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
    fn write_back(self, flush: rerail_blocks::block::Flush) -> StoreResult<Done> {
        match self {
            #[cfg(unix)]
            Done::Mapped(sealed, reader) => {
                let path = sealed.path().to_owned();
                let frozen = sealed.write_back(flush)?;
                drop(reader);
                Ok(Done::Written(frozen, path))
            }
            #[cfg(not(unix))]
            Done::Memory(frozen, path) => {
                let _ = flush;
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
        super::prepare_cache_directory(&self.dir)?;
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = self.dir.join(format!("w-{}-{n}.seg", std::process::id()));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let open = Open::Mapped(rerail_accel::gpu::shared::lease_deferrable_host(kind::JOURNAL, bytes, &file).map_err(StoreError::Invalid)?);
        #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
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

    /// No entries placed yet.
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
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
    /// Writes every segment back that is not yet (the writer's half; again after a failure, only those left), as the
    /// store's `flush` says (decision 0481).
    pub(super) fn write_back(&mut self, flush: rerail_blocks::block::Flush) -> StoreResult<()> {
        for d in &mut self.done {
            match d.take() {
                Some(segment) => *d = Some(segment.write_back(flush)?),
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
    /// Once the rows naming them are written: each segment renamed to its number (`numbers`, in `files`' order) in
    /// `dir`. A reader that found a row meanwhile misses. A compaction never finds those rows without their file, which
    /// would forget their entries (`SegmentKind::compact`): the store's writer runs every job as a savepoint of its
    /// batch's one immediate transaction (`in_one_transaction`), so the rename comes before the batch commits, and a
    /// compaction takes the store's write lock (`immediate_transaction_once`): it reads the rows only once they and
    /// their files are both there (`a_compaction_cannot_run_inside_the_batch_that_names_its_segments`). A write run
    /// with no writer (a handle that writes at once) is its run's, done while the run holds its collection lease,
    /// which every compaction waits out (`try_collect`, `collect_corpus_pass`).
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
pub(super) fn write_segments(dir: &Path, entries: &[(&[u8], i64, &[u8; 32])], flush: rerail_blocks::block::Flush) -> StoreResult<SealedSegments> {
    let mut w = SegmentWriter::new(dir, entries.len(), rerail_blocks::journal::reserve_for(entries.len(), entries.iter().map(|e| e.0.len()), ALIGN));
    for (bytes, key, digest) in entries {
        w.put(bytes.len(), *key, digest, |out| out.copy_from_slice(bytes))?;
    }
    let mut segments = w.seal();
    segments.write_back(flush)?;
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_underestimated_batch_grows_segment_directories() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = SegmentWriter::new(dir.path(), 1, ALIGN);
        for value in 0..32u64 {
            let bytes = value.to_le_bytes();
            writer.put(bytes.len(), value as i64, &[0; 32], |out| out.copy_from_slice(&bytes)).unwrap();
        }
        let mut segments = writer.seal();
        let files = segments.files().len();
        assert!(files <= 6, "expected at most 6 segments for 32 small entries, got {files}");
        segments.write_back(rerail_blocks::block::Flush::Synced).unwrap();
        for value in 0..32u64 {
            assert_eq!(segments.entry(value as usize).bytes(), value.to_le_bytes());
        }
    }
}
