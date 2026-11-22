//! Tapes and sources in segments (decision 0295 step 5): every source and tape the store keeps, as it is, placed in
//! a segment of `<store>.objects/` and read there in place, mapped; its row (`source_objects`, `tape_objects`) points
//! at it (`seg`, `seg_at`, `seg_len`, codec [`CODEC_RAW`], no frame). No row holds an object's bytes (store floor 7,
//! 2026-11-19): a write outside a run's waves is placed in a segment of its own ([`Placing`]).
//!
//! The run's parse workers add each file's source and tape to the run's gather ([`ObjectGather`], `write_back::Gather`),
//! held as the run holds them (no copy); once it holds the gather's cap, or the run's wave is sealed, it is placed in
//! one segment and queued as one job, its tapes read there meanwhile ([`QueuedTape`]).

use super::rows::{generation, intern_hash};
use super::segments::{ALIGN, Place, SealedSegments, SegmentBytes, SegmentMaps, SegmentWriter};
use super::write_back::{Closed, Gather, Held, Key, Kind, Syms, Work};
use super::{ContentStore, HashKey, SourceId, StoreError, StoreResult, TapeId, TapePayload};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The codec of a row whose bytes are in a segment, uncompressed.
pub(super) const CODEC_RAW: i64 = 0;

/// The directory of the store's tape and source segments: beside its file.
pub(super) fn object_dir(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".objects");
    PathBuf::from(name)
}

/// What the store at `from` keeps beside its file, copied beside the store at `to` (in place of any there): its atoms
/// blocks and its object segments. For a store file copied whole (a capture's, a replay's copy of it): without them
/// its rows name blocks and segments that are not there.
pub fn copy_beside(from: &Path, to: &Path) -> std::io::Result<()> {
    super::atoms::copy_blocks(from, to)?;
    let (segments, into) = (object_dir(from), object_dir(to));
    if into.exists() {
        std::fs::remove_dir_all(&into)?;
    }
    let Ok(entries) = std::fs::read_dir(&segments) else { return Ok(()) };
    std::fs::create_dir_all(&into)?;
    for e in entries.flatten() {
        super::atoms::clone_file(&e.path(), &into.join(e.file_name()))?;
    }
    Ok(())
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

/// A row's object: the range of a segment it names (`seg`, `seg_at`, `seg_len`). `None` when that segment is not
/// there or holds no such range (`seg` -1: it went missing): the object is absent, made again.
pub(super) fn row_frame(maps: &mut SegmentMaps, seg: i64, at: i64, len: i64) -> Option<ObjectFrame> {
    if seg <= 0 {
        return None;
    }
    maps.at(Place { seg, at: at as u64, len: len as usize }).map(ObjectFrame)
}

/// Objects a writer outside a run's waves stores (a standalone build's file, a publication's, a test's), as they
/// are, in one segment of the store's objects written back before the write's transaction: the transaction numbers
/// it (`number`), each row names its entry (`place`), and once the transaction commits the file takes its number
/// (`name`). A write that does not commit leaves no file (`SealedSegments`' drop). A run's files take the run's
/// gather instead (`ObjectGather`), the same rows over the same segments.
pub(super) struct Placing {
    dir: PathBuf,
    segments: Option<SealedSegments>,
}

impl Placing {
    /// `entries` (each an object's bytes) in one new segment of the store at `database`, written back as `flush`
    /// says (decision 0481); nothing for none.
    pub(super) fn write(database: &Path, entries: &[&[u8]], flush: rerail_blocks::block::Flush) -> StoreResult<Placing> {
        let dir = object_dir(database);
        if entries.is_empty() {
            return Ok(Placing { dir, segments: None });
        }
        let digest = [0u8; 32];
        let listed: Vec<(&[u8], i64, &[u8; 32])> = entries.iter().map(|b| (*b, 0, &digest)).collect();
        let segments = super::segments::write_segments(&dir, &listed, flush)?;
        if segments.files().len() != 1 {
            return Err(StoreError::Invalid("a write's objects made more than one segment".into()));
        }
        Ok(Placing { dir, segments: Some(segments) })
    }
    /// In the write's transaction: the segment's number, registered (`None` where nothing was placed).
    pub(super) fn number(&self, t: &Connection) -> StoreResult<Option<i64>> {
        if self.segments.is_none() {
            return Ok(None);
        }
        let seg: i64 = t.query_row("SELECT coalesce(max(seg), 0) + 1 FROM object_segments", [], |r| r.get(0))?;
        t.execute("INSERT INTO object_segments(seg) VALUES (?1)", [seg])?;
        Ok(Some(seg))
    }
    /// Entry `i`'s place, in the segment numbered `seg`.
    pub(super) fn place(&self, seg: i64, i: usize) -> Place {
        let (_, at, len) = self.segments.as_ref().expect("a placed segment").placed(i);
        Place { seg, at: at as u64, len }
    }
    /// Once the transaction that numbered it (`seg`) commits: the file takes its number.
    pub(super) fn name(&mut self, seg: Option<i64>) -> StoreResult<()> {
        match (&mut self.segments, seg) {
            (Some(segments), Some(seg)) => segments.name(&self.dir, &[seg]),
            _ => Ok(()),
        }
    }
}

/// The store's segments, mapped as rows name them, for a reader on a connection of its own (a SQL module's).
pub struct ObjectMaps(SegmentMaps);

impl ObjectMaps {
    /// The segments of the store at `database`.
    pub fn new(database: &Path) -> ObjectMaps {
        ObjectMaps(SegmentMaps::new(object_dir(database)))
    }
    /// A row's object (`seg`, `seg_at`, `seg_len`), as [`row_frame`].
    pub fn frame(&mut self, seg: i64, at: i64, len: i64) -> Option<ObjectFrame> {
        row_frame(&mut self.0, seg, at, len)
    }
}

/// A tape (with its source) this process queued and the store has not written: read in memory until it is placed,
/// then in its segment (`ContentStore::queued_tape`).
pub struct QueuedTape(pub ObjectBytes);


pub(super) struct Item {
    key: Key,
    canonical: HashKey,
    grammar_id: i64,
    grammar_version: i64,
    tape_format_version: i64,
    source: Arc<String>,
    tape: TapeBytes,
    tape_identity: HashKey,
    /// Whether the run's manifest names the source once written (`ContentStore::name_sources`).
    named: bool,
    /// The file's programs (decision 0431 step 6), recorded with its tape; `None`: not known, none recorded.
    documents: Option<Arc<[super::QueuedDocument]>>,
}

/// A run's parsed files gathered for the store: placed in one segment when closed.
pub(super) struct ObjectGather {
    dir: PathBuf,
    items: Vec<Item>,
    at: HashMap<Key, usize>,
    bytes: usize,
}

/// What the writer inserts of a placed file: its source's and tape's entries are `2 i` and `2 i + 1`.
pub(super) struct Row {
    canonical: HashKey,
    grammar_id: i64,
    grammar_version: i64,
    tape_format_version: i64,
    source_len: usize,
    tape_len: usize,
    tape_identity: HashKey,
    named: bool,
    documents: Option<Arc<[super::QueuedDocument]>>,
}

impl ObjectGather {
    fn add(&mut self, item: Item) {
        if self.at.contains_key(&item.key) {
            return;
        }
        self.bytes += item.source.len() + (*item.tape).as_ref().len();
        self.at.insert(item.key, self.items.len());
        self.items.push(item);
    }
    fn held(item: &Item) -> Held {
        Arc::new(QueuedTape(ObjectBytes::Tape(Arc::clone(&item.tape))))
    }
}

impl Gather for ObjectGather {
    fn keys(&self) -> Vec<(Key, Option<Held>)> {
        self.items.iter().map(|i| (i.key, Some(Self::held(i)))).collect()
    }
    fn held(&self, key: &Key) -> Option<Held> {
        self.at.get(key).map(|&i| Self::held(&self.items[i]))
    }
    fn bytes(&self) -> usize {
        self.bytes
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn close(self: Box<Self>) -> Option<Closed> {
        if self.items.is_empty() {
            return None;
        }
        let n = self.items.len() * 2;
        let lens = self.items.iter().flat_map(|i| [i.source.len(), (*i.tape).as_ref().len()]);
        let mut w = SegmentWriter::new(&self.dir, n, rerail_blocks::journal::reserve_for(n, lens, ALIGN));
        let mut placed = true;
        for (i, item) in self.items.iter().enumerate() {
            let key = i as i64;
            let digest = item.canonical.sha256_bytes().unwrap_or_default();
            let (source, tape) = (item.source.as_bytes(), (*item.tape).as_ref());
            if w.put(source.len(), key, &digest, |out| out.copy_from_slice(source)).is_err() || w.put(tape.len(), key, &digest, |out| out.copy_from_slice(tape)).is_err() {
                placed = false;
                break;
            }
        }
        let segments = w.seal();
        let what = "tapes and their sources";
        if !placed {
            // No room for a segment (the disk): the files are not stored, and a later run parses them again, as a
            // write the full queue drops is.
            drop(segments);
            return None;
        }
        let keys = self.items.iter().enumerate().map(|(i, item)| (item.key, Some(Arc::new(QueuedTape(ObjectBytes::Mapped(segments.entry(2 * i + 1)))) as Held))).collect();
        let rows: Vec<Row> = self
            .items
            .into_iter()
            .map(|i| Row {
                canonical: i.canonical,
                grammar_id: i.grammar_id,
                grammar_version: i.grammar_version,
                tape_format_version: i.tape_format_version,
                source_len: i.source.len(),
                tape_len: (*i.tape).as_ref().len(),
                tape_identity: i.tape_identity,
                named: i.named,
                documents: i.documents,
            })
            .collect();
        // The queue holds the rows; the bytes are the OS's pages.
        let bytes = rows.len() * 96;
        Some(Closed { what, bytes, keys, syms: Syms::None, now: None, job: ObjectsWrite { rows, segments }.work() })
    }
}

/// Sources and tapes as the writer writes them (decision 0306 §8): a closed gather's, in its segments.
pub(super) struct ObjectsWrite {
    rows: Vec<Row>,
    segments: SealedSegments,
}

impl ObjectsWrite {
    fn work(self) -> Work {
        Work(Kind::Objects(Box::new(self)))
    }

    pub(super) fn run(&mut self, store: &ContentStore) -> StoreResult<()> {
        let ids = store.put_objects_placed(&self.rows, &mut self.segments)?;
        // A file's programs, recorded with its tape once it is (decision 0431 step 6).
        for (row, id) in self.rows.iter().zip(&ids) {
            if let Some(documents) = row.documents.as_ref().filter(|d| !d.is_empty()) {
                store.put_programs(*id, (row.grammar_id, row.grammar_version, row.tape_format_version), &documents.iter().map(super::QueuedDocument::payload).collect::<Vec<_>>())?;
            }
        }
        store.name_sources(self.rows.iter().zip(ids).filter(|(row, _)| row.named).map(|(_, id)| id));
        Ok(())
    }
}

impl ContentStore {
    /// Queues a parsed file's source and tape for the store, behind the run (decision 0104), under `key` (a read of
    /// the tape meanwhile: [`Self::queued_tape`]): gathered with the run's others into one segment (0295 step 5), or,
    /// outside a run's waves, compressed here and queued alone. `named`: the run's manifest names the source once it is
    /// written (`name_sources`).
    /// Where the queue is full a lone write is dropped, and the next run parses the file again.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_source_and_tape(&self, key: Key, canonical: HashKey, grammar_id: i64, grammar_version: i64, tape_format_version: i64, source: &Arc<String>, tape: TapeBytes, named: bool, documents: Option<Arc<[super::QueuedDocument]>>) -> StoreResult<()> {
        let tape_identity = HashKey::tape_identity(canonical, (*tape).as_ref());
        let mut item = Some(Item { key, canonical, grammar_id, grammar_version, tape_format_version, source: Arc::clone(source), tape, tape_identity, named, documents });
        let dir = object_dir(&self.database_path);
        let make = || ObjectGather { dir: dir.clone(), items: Vec::new(), at: HashMap::new(), bytes: 0 };
        if self.gather("tapes and their sources", make, |g| g.add(item.take().expect("the file"))) {
            return Ok(());
        }
        // Outside a run's waves: a gather of this one file, placed in a segment of its own now, written as the run's.
        let mut gather = make();
        gather.add(item.take().expect("the file, not gathered"));
        if let Some(c) = Box::new(gather).close() {
            self.write_back_unless_full(c.syms, c.what, c.bytes, c.keys, c.job);
        }
        Ok(())
    }

    /// The tape this process queued under `key` and the store has not written yet: gathered or placed.
    pub fn queued_tape(&self, key: &Key) -> Option<StoreResult<ObjectBytes>> {
        self.pending::<QueuedTape>(key).map(|q| Ok(q.0.clone()))
    }

    /// Writes a closed gather's files: its segments written back and numbered, each source and tape a row pointing at
    /// its entry (one already stored keeps its row), in one transaction; the segments named once it commits. Run
    /// again whole after a busy store: written back once, numbered again. The sources' ids, in `rows`' order.
    fn put_objects_placed(&self, rows: &[Row], segments: &mut SealedSegments) -> StoreResult<Vec<SourceId>> {
        self.require_write_capacity()?;
        segments.write_back(self.connection.flush())?;
        let files = segments.files();
        let segs: &SealedSegments = segments;
        let (ids, numbers) = self.immediate_transaction(|t| {
            let created = generation(t)?;
            let mut numbers: Vec<i64> = Vec::with_capacity(files.len());
            for _ in &files {
                let seg: i64 = t.query_row("SELECT coalesce(max(seg), 0) + 1 FROM object_segments", [], |r| r.get(0))?;
                t.execute("INSERT INTO object_segments(seg) VALUES (?1)", [seg])?;
                numbers.push(seg);
            }
            let place = |i: usize| {
                let (s, at, len) = segs.placed(i);
                Place { seg: numbers[s], at: at as u64, len }
            };
            let mut ids = Vec::with_capacity(rows.len());
            for (i, row) in rows.iter().enumerate() {
                let source = put_source_placed_tx(t, row.canonical, row.source_len, place(2 * i), created)?;
                let tape = TapePayload { grammar_id: row.grammar_id, grammar_version: row.grammar_version, tape_format_version: row.tape_format_version, bytes: &[], documents: &[] };
                let tape_id = put_tape_placed_tx(t, source, tape, row.tape_len, row.tape_identity, place(2 * i + 1), created)?;
                // A file that holds no program records so with its tape (decision 0431 step 6), in this transaction;
                // one that holds some has them written after (`put_programs`).
                if row.documents.as_ref().is_some_and(|d| d.is_empty()) {
                    t.prepare_cached("UPDATE tape_objects SET programs=0 WHERE tape_id=?1 AND programs<0")?.execute([tape_id.0])?;
                }
                ids.push(source);
            }
            Ok((ids, numbers))
        })?;
        segments.name(&object_dir(&self.database_path), &numbers)?;
        Ok(ids)
    }

    /// Keeps `sources` (each its bytes, its sha256 and its aliases) placed in one new segment of the store's
    /// objects, uncompressed, to be read there in place (decision 0394): one write of the segment, one transaction
    /// for their rows, the segment named once it commits. A source whose segment went missing is moved there; one it
    /// holds placed keeps its place. Their ids, in order.
    pub fn put_sources_placed(&self, sources: &[(&[u8], HashKey, &[HashKey])]) -> StoreResult<Vec<SourceId>> {
        self.require_write_capacity()?;
        let held = self.sources_placed(&sources.iter().map(|s| s.1).collect::<Vec<_>>())?;
        let new: Vec<usize> = (0..sources.len()).filter(|&i| held[i].is_none()).collect();
        let digests: Vec<[u8; 32]> = new.iter().map(|&i| sources[i].1.sha256_bytes().ok_or_else(|| StoreError::Invalid("a placed source's hash is not a sha256".into()))).collect::<StoreResult<_>>()?;
        let dir = object_dir(&self.database_path);
        let mut segments = if new.is_empty() {
            None
        } else {
            let entries: Vec<(&[u8], i64, &[u8; 32])> = new.iter().zip(&digests).map(|(&i, d)| (sources[i].0, 0, d)).collect();
            Some(super::segments::write_segments(&dir, &entries, self.connection.flush())?)
        };
        let files = segments.as_ref().map_or_else(Vec::new, SealedSegments::files);
        let segs = segments.as_ref();
        let (ids, numbers) = self.immediate_transaction(|t| {
            let created = generation(t)?;
            let mut numbers: Vec<i64> = Vec::with_capacity(files.len());
            for _ in &files {
                let seg: i64 = t.query_row("SELECT coalesce(max(seg), 0) + 1 FROM object_segments", [], |r| r.get(0))?;
                t.execute("INSERT INTO object_segments(seg) VALUES (?1)", [seg])?;
                numbers.push(seg);
            }
            let mut ids = Vec::with_capacity(sources.len());
            let mut k = 0;
            for (i, (bytes, canonical, aliases)) in sources.iter().enumerate() {
                let id = match held[i] {
                    Some(id) => id,
                    None => {
                        let (s, at, len) = segs.expect("a segment for the new sources").placed(k);
                        k += 1;
                        put_source_placed_tx(t, *canonical, bytes.len(), Place { seg: numbers[s], at: at as u64, len }, created)?
                    }
                };
                for alias in *aliases {
                    let alias_id = intern_hash(t, *alias)?;
                    let was: Option<i64> = t.prepare_cached("SELECT source_id FROM source_aliases WHERE alias_hash_id = ?1")?.query_row([alias_id], |r| r.get(0)).optional()?;
                    if was.is_some_and(|was| was != id.0) {
                        return Err(StoreError::Corrupt("an alias maps to different source bytes".to_owned()));
                    }
                    t.prepare_cached("INSERT INTO source_aliases(alias_hash_id, source_id) VALUES (?1, ?2) ON CONFLICT(alias_hash_id) DO NOTHING")?.execute(params![alias_id, id.0])?;
                }
                ids.push(id);
            }
            Ok((ids, numbers))
        })?;
        if let Some(segments) = &mut segments {
            segments.name(&dir, &numbers)?;
        }
        Ok(ids)
    }

    /// Each of `hashes`' source, where the store holds it in a segment: its id, else `None` (not held, or its segment
    /// went missing).
    fn sources_placed(&self, hashes: &[HashKey]) -> StoreResult<Vec<Option<SourceId>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached(
            "SELECT s.source_id FROM hashes h JOIN source_objects s ON s.canonical_hash_id = h.hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 AND s.seg > 0",
        )?;
        hashes.iter().map(|h| st.query_row(super::rows::hash_params(*h), |r| r.get::<_, i64>(0)).optional().map(|v| v.map(SourceId)).map_err(StoreError::from)).collect()
    }

    /// The source each of `aliases` names, in their order, in one read (decision 0394): read in its segment, mapped
    /// (`ObjectBytes::in_place`): not decompressed, hashed or copied, its segment's check (when mapped) the check.
    /// `None` where no source has the alias, or its segment is gone.
    pub fn sources_by_alias_in_place(&self, aliases: &[HashKey]) -> StoreResult<Vec<Option<ObjectBytes>>> {
        let rows: Vec<Option<(i64, [i64; 3], i64)>> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(
                "SELECT s.raw_length, s.seg, s.seg_at, s.seg_len, s.source_id \
                 FROM hashes h JOIN source_aliases a ON a.alias_hash_id = h.hash_id JOIN source_objects s ON s.source_id = a.source_id \
                 WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5",
            )?;
            aliases
                .iter()
                .map(|alias| {
                    st.query_row(super::rows::hash_params(*alias), |r| {
                        Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?], r.get(4)?))
                    })
                    .optional()
                    .map_err(StoreError::from)
                })
                .collect::<StoreResult<_>>()?
        };
        // Archived ones (decision 0450 step 4) are placed back first, then read in place.
        if self.placed_again(rows.iter().flatten().map(|&(_, [seg, ..], source)| (super::reclamation::OBJECT_SOURCE, source, seg)))? {
            return self.sources_by_alias_in_place(aliases);
        }
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Some((raw_length, [seg, seg_at, seg_len], _)) = row else {
                out.push(None);
                continue;
            };
            let length = usize::try_from(raw_length).map_err(|_| StoreError::Corrupt("invalid source length".into()))?;
            let Some(frame) = row_frame(&mut maps, seg, seg_at, seg_len) else {
                out.push(None);
                continue;
            };
            out.push(Some(frame.bytes(length).map_err(|e| StoreError::Corrupt(format!("a source's bytes: {e}")))?));
        }
        Ok(out)
    }

    /// The source `alias` names, read where the store holds it (`sources_by_alias_in_place`).
    pub fn source_by_alias_in_place(&self, alias: HashKey) -> StoreResult<Option<ObjectBytes>> {
        Ok(self.sources_by_alias_in_place(&[alias])?.pop().flatten())
    }

    /// The object segments' bytes, and the bytes their rows name (`PLACED`): what compaction would leave.
    pub fn object_segment_bytes(&self) -> StoreResult<(u64, u64)> {
        let usage = self.object_segment_use()?;
        Ok((usage.held, usage.named))
    }

    /// The object segments' bytes, the bytes their rows name (`PLACED`), and their numbers.
    fn object_segment_use(&self) -> StoreResult<super::segments::SegmentUse> {
        let dir = object_dir(&self.database_path);
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(object_use(&connection, &object_kind(&dir))?)
    }

    /// Merges the object segments compaction retires (`segments::retired`): every one where the bytes no row names
    /// (objects stored again, collected) are more than half of them and at least `floor`, else, past
    /// `segments::SEGMENT_BUDGET` of them, all but the oldest half of it; one write transaction. The placed objects
    /// there (each table of `PLACED`) are copied, bytes several rows name once. A row whose segment is not there is
    /// marked so where others name it (a source or tape: `seg` -1, never a segment's number, absent to readers until
    /// its file is parsed and stored again), and removed where it is a cache (a result, a page: computed again).
    /// Returns the bytes freed. A reader that mapped an old segment reads it on.
    pub fn compact_object_segments(&self, floor: u64) -> StoreResult<u64> {
        self.compact_object_segments_within(floor, super::segments::SEGMENT_BUDGET)
    }

    /// `compact_object_segments`, keeping at most `budget` segments. Chosen again in the write transaction
    /// (`SegmentKind::compact`); read before it too, so a store with nothing due takes no write lock.
    fn compact_object_segments_within(&self, floor: u64, budget: usize) -> StoreResult<u64> {
        let usage = self.object_segment_use()?;
        if super::segments::retired(&usage, floor, budget).is_none() {
            return Ok(0);
        }
        let dir = object_dir(&self.database_path);
        let kind = object_kind(&dir);
        let retired = self.immediate_transaction_once(|t| kind.compact(t, &object_use(t, &kind)?, (floor, budget), placed_objects, |o| lose_object(t, o), |o, seg, at| move_object(t, o, seg, at)))?;
        kind.remove(&retired);
        Ok(usage.held.saturating_sub(self.object_segment_bytes()?.0))
    }
}

/// The object segments, as compaction finds and merges them.
fn object_kind(dir: &Path) -> super::segments::SegmentKind<'_> {
    super::segments::SegmentKind { table: "object_segments", column: "seg", dir }
}

/// The object segments' use, read in `t` (`segments::SegmentUse`): the bytes every table of `PLACED` names.
fn object_use(t: &Connection, kind: &super::segments::SegmentKind<'_>) -> rusqlite::Result<super::segments::SegmentUse> {
    let sum = PLACED.iter().map(|p| format!("(SELECT coalesce(sum(seg_len), 0) FROM {} WHERE seg > 0)", p.table)).collect::<Vec<_>>().join(" + ");
    kind.usage(t, &format!("SELECT {sum}"))
}

/// A placed row, as compaction moves it: its table (of `PLACED`) and its key's values.
type PlacedRow = (usize, Vec<rusqlite::types::Value>);

/// The rows of every table of `PLACED` placed in segments numbered `from` and after, each table's in their order
/// there.
fn placed_objects(t: &Connection, from: i64) -> rusqlite::Result<Vec<super::segments::Moved<PlacedRow>>> {
    let mut out = Vec::new();
    for (table, placed) in PLACED.iter().enumerate() {
        let sql = format!("SELECT {}, seg, seg_at, seg_len FROM {} WHERE seg > 0 AND seg >= ?1 ORDER BY seg, seg_at", placed.key.join(", "), placed.table);
        let n = placed.key.len();
        let mut st = t.prepare(&sql)?;
        let mut q = st.query([from])?;
        while let Some(r) = q.next()? {
            let key = (0..n).map(|c| r.get::<_, rusqlite::types::Value>(c)).collect::<Result<Vec<_>, _>>()?;
            let place = Place { seg: r.get(n)?, at: r.get::<_, i64>(n + 1)? as u64, len: r.get::<_, i64>(n + 2)? as usize };
            out.push(super::segments::Moved { key: (table, key), doc: 0, digest: [0; 32], place });
        }
    }
    Ok(out)
}

/// A placed row whose segment is not there: marked so (`seg` -1), as other rows name it.
fn lose_object(t: &Connection, (table, key): &PlacedRow) -> rusqlite::Result<()> {
    let placed = &PLACED[*table];
    let sql = format!("UPDATE {} SET seg = -1 WHERE {}", placed.table, key_filter(placed.key, 1));
    t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(key.iter())).map(drop)
}

/// A placed row's bytes, moved to segment `seg` at `at`.
fn move_object(t: &Connection, (table, key): &PlacedRow, seg: i64, at: i64) -> rusqlite::Result<()> {
    let spec = &PLACED[*table];
    let sql = format!("UPDATE {} SET seg = ?1, seg_at = ?2 WHERE {}", spec.table, key_filter(spec.key, 3));
    let mut values: Vec<rusqlite::types::Value> = vec![seg.into(), at.into()];
    values.extend(key.iter().cloned());
    t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(values.iter())).map(drop)
}

/// A table whose rows name their bytes' place in an object segment (`seg`, `seg_at`, `seg_len`), with its key.
struct PlacedTable {
    table: &'static str,
    key: &'static [&'static str],
}

/// Every table whose rows name a place in the object segments: sources and tapes (decision 0295 step 5).
const PLACED: [PlacedTable; 2] = [
    PlacedTable { table: "tape_objects", key: &["tape_id"] },
    PlacedTable { table: "source_objects", key: &["source_id"] },
];

/// `k1 = ?n AND k2 = ?n+1 …` over `key`, its parameters from `first`.
fn key_filter(key: &[&str], first: usize) -> String {
    key.iter().enumerate().map(|(i, k)| format!("{k} = ?{}", first + i)).collect::<Vec<_>>().join(" AND ")
}

/// A source whose bytes are at `place`: inserted, or (stored by its hash) moved there, so a row whose segment went
/// missing is whole again once its file is parsed again; its id either way.
fn put_source_placed_tx(t: &Connection, canonical: HashKey, len: usize, place: Place, created: i64) -> StoreResult<SourceId> {
    let hash = intern_hash(t, canonical)?;
    t.prepare_cached(
        "INSERT INTO source_objects(canonical_hash_id,raw_length,codec_id,frame,created_generation,seg,seg_at,seg_len) \
         VALUES (?1,?2,?3,x'',?4,?5,?6,?7) ON CONFLICT(canonical_hash_id) DO UPDATE SET codec_id=excluded.codec_id, \
         frame=x'', seg=excluded.seg, seg_at=excluded.seg_at, seg_len=excluded.seg_len",
    )?
    .execute(params![hash, len as i64, CODEC_RAW, created, place.seg, place.at as i64, place.len as i64])?;
    let id: i64 = t.prepare_cached("SELECT source_id FROM source_objects WHERE canonical_hash_id=?1")?.query_row([hash], |r| r.get(0))?;
    Ok(SourceId(id))
}

/// A tape of `source` whose bytes are at `place`: inserted, or (stored by its source, grammar and versions) moved
/// there, as a source is; its id either way.
fn put_tape_placed_tx(t: &Connection, source: SourceId, tape: TapePayload<'_>, len: usize, canonical: HashKey, place: Place, created: i64) -> StoreResult<TapeId> {
    super::objects::validate_tape_payload(Some(tape))?;
    let hash = intern_hash(t, canonical)?;
    t.prepare_cached(
        "INSERT INTO tape_objects(source_id,canonical_hash_id,grammar_id,grammar_version,tape_format_version,codec_id,raw_length,frame,created_generation,seg,seg_at,seg_len) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,x'',?8,?9,?10,?11) ON CONFLICT(source_id,grammar_id,grammar_version,tape_format_version) \
         DO UPDATE SET codec_id=excluded.codec_id, frame=x'', seg=excluded.seg, seg_at=excluded.seg_at, seg_len=excluded.seg_len",
    )?
    .execute(params![source.0, hash, tape.grammar_id, tape.grammar_version, tape.tape_format_version, CODEC_RAW, len as i64, created, place.seg, place.at as i64, place.len as i64])?;
    let id: i64 = t
        .prepare_cached("SELECT tape_id FROM tape_objects WHERE source_id=?1 AND grammar_id=?2 AND grammar_version=?3 AND tape_format_version=?4")?
        .query_row(params![source.0, tape.grammar_id, tape.grammar_version, tape.tape_format_version], |r| r.get(0))?;
    Ok(TapeId(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_store::write_back;

    fn tape_of(store: &ContentStore, source: SourceId) -> Option<Vec<u8>> {
        store.tape_frames_for(&[source], 7, 1, 1).unwrap().remove(0).map(|f| f.bytes().unwrap().to_vec())
    }

    /// Object segments past a budget are merged though their bytes are named: all but the oldest half of the budget,
    /// into one numbered after them, every source reading the same from it.
    #[test]
    fn object_segments_past_the_budget_are_merged() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let sources: Vec<Vec<u8>> = (0..12u8).map(|i| vec![i + 1; 4000]).collect();
        let ids: Vec<SourceId> = sources.iter().map(|s| store.put_source(s, &[]).unwrap()).collect();
        // The segments' numbers, oldest first.
        let segments = || -> Vec<i64> {
            let mut n: Vec<i64> = std::fs::read_dir(object_dir(&store.database_path)).unwrap().map(|e| e.unwrap().path().file_stem().unwrap().to_string_lossy().parse().unwrap()).collect();
            n.sort_unstable();
            n
        };
        let before = segments();
        assert_eq!(before.len(), 12, "a segment a source put alone");
        assert_eq!(store.compact_object_segments(0).unwrap(), 0, "named, and fewer than the budget");
        assert_eq!(store.compact_object_segments_within(u64::MAX, 12).unwrap(), 0, "no more than the budget");
        store.compact_object_segments_within(u64::MAX, 8).unwrap();
        assert_eq!(segments(), [&before[..4], &[before[11] + 1]].concat(), "the oldest four left, the newer eight merged into one after them");
        assert_eq!(store.compact_object_segments_within(u64::MAX, 8).unwrap(), 0, "within the budget now");
        for (id, source) in ids.iter().zip(&sources) {
            assert_eq!(&store.get_source(*id).unwrap(), source, "source {id:?} read from its segment");
        }
    }

    /// Decision 0394: sources put placed are kept in one new segment, uncompressed, and read there by alias, mapped:
    /// not decompressed, hashed or copied. A source put alone is in a segment of its own too (no row holds bytes,
    /// floor 7); one held placed keeps its place and id (no new segment); an alias of other bytes is refused.
    #[test]
    fn sources_put_placed_are_read_in_place_by_alias() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let alias = |n: u8| HashKey::sha256(&[b"a test's alias ", &[n][..]].concat());
        let (old, new) = (vec![7u8; 5000], b"a run's rows".repeat(300));
        let old_id = store.put_source(&old, &[alias(0)]).unwrap();
        let read = |aliases: &[HashKey]| store.sources_by_alias_in_place(aliases).unwrap();
        let alone = read(&[alias(0)]).remove(0).expect("held");
        assert!(alone.in_place() && *alone == old[..], "put alone: read in place in its segment");
        let segs = |store: &ContentStore| store.connection.lock().unwrap().query_row("SELECT count(*) FROM object_segments", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(segs(&store), 1, "a segment of its own");
        let ids = store.put_sources_placed(&[(&old, HashKey::sha256(&old), &[]), (&new, HashKey::sha256(&new), &[alias(1)])]).unwrap();
        assert_eq!(ids[0], old_id, "held placed: its id kept");
        assert_eq!(segs(&store), 2, "one segment for the new one");
        let got = read(&[alias(0), alias(1), alias(2)]);
        assert!(got[2].is_none(), "no source has that alias");
        for (b, want) in got.iter().zip([&old[..], &new[..]]) {
            let b = b.as_ref().expect("placed");
            assert!(matches!(b, ObjectBytes::Mapped(_)) && b.in_place(), "read in place, mapped");
            assert_eq!(&**b, want);
            assert_eq!(b.as_ptr() as usize % 16, 0, "at 16 bytes, as an image is read");
        }
        let rows: Vec<(i64, i64, i64)> = store.connection.lock().unwrap().prepare("SELECT codec_id, seg, length(frame) FROM source_objects").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
        assert!(rows.iter().all(|&(codec, seg, frame)| codec == CODEC_RAW && seg > 0 && frame == 0), "pointers, no frames: {rows:?}");
        // Put again: the same ids, no new segment.
        assert_eq!(store.put_sources_placed(&[(&new, HashKey::sha256(&new), &[alias(1)])]).unwrap(), vec![ids[1]]);
        assert_eq!(segs(&store), 2, "held placed: no new segment");
        // An alias of other bytes.
        assert!(store.put_sources_placed(&[(&old, HashKey::sha256(&old), &[alias(1)])]).is_err(), "an alias of other bytes is refused");
        // A new process reads them in place too.
        drop(store);
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        assert!(store.sources_by_alias_in_place(&[alias(0), alias(1)]).unwrap().iter().all(|b| b.as_ref().is_some_and(ObjectBytes::in_place)));
    }

    /// A test's files: its sources, each with its tape.
    type Files = [(Arc<String>, Arc<Vec<u8>>)];

    fn key(i: usize) -> Key {
        Key::of(&[b"a test's tape", &[i as u8]])
    }

    /// `files` queued in one run: gathered (the first read where the run holds it), until the gather is full (the
    /// last): placed, read in its segment, the wave not sealed yet.
    fn queue(store: &ContentStore, files: &Files) {
        let _waves = write_back::run_waves();
        for (i, (source, tape)) in files.iter().enumerate() {
            store.queue_source_and_tape(key(i), HashKey::sha256(source.as_bytes()), 7, 1, 1, source, Arc::clone(tape) as TapeBytes, true, None).unwrap();
            match i {
                0 => assert!(matches!(store.queued_tape(&key(0)), Some(Ok(ObjectBytes::Tape(_))))),
                3 => assert!(matches!(store.queued_tape(&key(0)), Some(Ok(ObjectBytes::Mapped(_))))),
                _ => {}
            }
        }
    }

    /// Each of `files` reads back whole: its tape and its source.
    fn assert_read_back(store: &ContentStore, ids: &[SourceId], files: &Files) {
        for (id, (source, tape)) in ids.iter().zip(files) {
            assert_eq!(tape_of(store, *id).as_deref(), Some(&tape[..]));
            assert_eq!(store.get_source(*id).unwrap(), source.as_bytes());
        }
    }

    /// Every tape and source row points into a segment and holds no frame.
    fn assert_pointers(store: &ContentStore) {
        let rows: Vec<(i64, i64, i64)> = store.connection.lock().unwrap().prepare("SELECT codec_id, seg, length(frame) FROM tape_objects UNION ALL SELECT codec_id, seg, length(frame) FROM source_objects").unwrap().query_map([], |r| <(i64, i64, i64)>::try_from(r)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|&(codec, seg, frame)| (codec, seg > 0, frame) == (CODEC_RAW, true, 0)), "pointers, no frames: {rows:?}");
    }

    /// Decision 0295 step 5: a run's parsed files are gathered, placed in one segment once the gather is full or the
    /// wave is sealed (read there meanwhile), and written as rows pointing at it: no frame, read in place. A row whose
    /// segment went missing is a miss, whole again once its file is stored again; stored again, the old bytes are no
    /// row's, and compaction leaves one segment of the named ones.
    #[test]
    fn a_runs_tapes_and_sources_are_gathered_into_a_segment_and_read_there() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let big = write_back::gather_cap() + 1;
        let files: Vec<(Arc<String>, Arc<Vec<u8>>)> = (0..4).map(|i| (Arc::new(format!("source {i}\n")), Arc::new(vec![i as u8 + 1; if i == 3 { big } else { 100 + i }]))).collect();
        queue(&store, &files);
        store.written().unwrap();
        let ids: Vec<SourceId> = files.iter().map(|(source, _)| store.source_by_hash(HashKey::sha256(source.as_bytes())).unwrap().expect("stored")).collect();
        assert_eq!(ids.len(), 4);
        assert_pointers(&store);
        assert_read_back(&store, &ids, &files);
        let (held, named) = store.object_segment_bytes().unwrap();
        assert!(held >= named && named as usize > big, "{held} {named}");
        assert!(store.capacity().unwrap().bytes >= held, "the disk budget counts the segments");
        // A segment gone: a miss, until the file is stored again.
        std::fs::remove_dir_all(object_dir(&store.database_path)).unwrap();
        assert_eq!(tape_of(&store, ids[0]), None);
        queue(&store, &files);
        store.written().unwrap();
        assert_eq!(tape_of(&store, ids[0]).as_deref(), Some(&files[0].1[..]));
        // Stored again: the first copy is no row's, and compaction keeps one segment.
        queue(&store, &files);
        store.written().unwrap();
        assert!(store.compact_object_segments(0).unwrap() > 0);
        assert_eq!(std::fs::read_dir(object_dir(&store.database_path)).unwrap().count(), 1, "one segment left");
        assert_read_back(&store, &ids, &files);
    }
}
