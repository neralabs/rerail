//! Tapes and sources in segments (decision 0295 step 5): a run's parsed files, their sources and tapes uncompressed,
//! placed in segments of `<store>.objects/` and read there in place, mapped; their rows (`source_objects`,
//! `tape_objects`) point at them (`seg`, `seg_at`, `seg_len`, codec [`CODEC_RAW`], no frame). A row of `seg` 0 holds
//! its bytes inline, zstd-compressed (`frame`): written outside a run's waves, or before.
//!
//! The run's parse workers add each file's source and tape to the run's gather ([`ObjectGather`], `write_back::Gather`),
//! held as the run holds them (no copy); once it holds the gather's cap, or the run's wave is sealed, it is placed in
//! one segment and queued as one job, its tapes read there meanwhile ([`QueuedTape`]).

use super::rows::{generation, intern_hash};
use super::segments::{ALIGN, Place, SealedSegments, SegmentBytes, SegmentMaps, SegmentWriter};
use super::write_back::{Closed, Gather, Held, Key, Kind, Syms, Work};
use super::{CODEC_ZSTD, ContentStore, HashKey, SourceId, StoreError, StoreResult, TapeId, TapePayload};
use rusqlite::{Connection, params};
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

/// A stored object's bytes: decompressed into memory (a row of its own), or where they are in a segment, mapped.
#[derive(Clone)]
pub enum ObjectBytes {
    Heap(Arc<Vec<u8>>),
    Mapped(SegmentBytes),
}

impl std::ops::Deref for ObjectBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ObjectBytes::Heap(b) => b,
            ObjectBytes::Mapped(b) => b.bytes(),
        }
    }
}

impl ObjectBytes {
    /// Its bytes as a vector of its own: a decompressed one as it is, a mapped one copied.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            ObjectBytes::Heap(b) => Arc::try_unwrap(b).unwrap_or_else(|b| (*b).clone()),
            ObjectBytes::Mapped(b) => b.bytes().to_vec(),
        }
    }
}

impl std::fmt::Debug for ObjectBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectBytes({} bytes)", self.len())
    }
}

/// A stored object as its row has it: a zstd frame (inline), or its bytes in a segment.
#[derive(Clone, Debug)]
pub enum ObjectFrame {
    Zstd(Arc<Vec<u8>>),
    Raw(SegmentBytes),
}

impl ObjectFrame {
    /// The bytes it takes as stored.
    pub fn len(&self) -> usize {
        match self {
            ObjectFrame::Zstd(f) => f.len(),
            ObjectFrame::Raw(b) => b.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Its bytes: a frame decompressed, a segment's read in place.
    pub fn bytes(&self, raw_length: usize) -> std::io::Result<ObjectBytes> {
        match self {
            ObjectFrame::Zstd(f) => super::compression::decompress(f, raw_length).map(|b| ObjectBytes::Heap(Arc::new(b))),
            ObjectFrame::Raw(b) => Ok(ObjectBytes::Mapped(b.clone())),
        }
    }
    /// Its first `n` bytes.
    pub fn head(&self, n: usize) -> std::io::Result<Vec<u8>> {
        match self {
            ObjectFrame::Zstd(f) => super::compression::decompress_head(f, n),
            ObjectFrame::Raw(b) => Ok(b.bytes()[..n.min(b.len())].to_vec()),
        }
    }
    /// Its bytes, a chunk at a time.
    pub fn chunks(&self, mut each: impl FnMut(&[u8])) -> std::io::Result<()> {
        match self {
            ObjectFrame::Zstd(f) => super::compression::decompress_chunks(f, each),
            ObjectFrame::Raw(b) => {
                each(b.bytes());
                Ok(())
            }
        }
    }
    /// Its bytes into `out`, which is their length.
    pub fn copy_into(&self, out: &mut [u8]) -> std::io::Result<()> {
        match self {
            ObjectFrame::Zstd(f) => super::compression::decompress_into(f, out),
            ObjectFrame::Raw(b) if b.len() == out.len() => {
                out.copy_from_slice(b.bytes());
                Ok(())
            }
            ObjectFrame::Raw(_) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "an object's length")),
        }
    }
}

/// A row's object: its `frame` (codec `codec`), or the range of a segment it names. `None` when that segment is not
/// there or holds no such range: the object is absent, made again.
pub(super) fn row_frame(maps: &mut SegmentMaps, codec: i64, frame: Vec<u8>, seg: i64, at: i64, len: i64) -> StoreResult<Option<ObjectFrame>> {
    if seg != 0 {
        return Ok(maps.at(Place { seg, at: at as u64, len: len as usize }).map(ObjectFrame::Raw));
    }
    if codec != CODEC_ZSTD {
        return Err(StoreError::Corrupt(format!("unsupported object codec {codec}")));
    }
    Ok(Some(ObjectFrame::Zstd(Arc::new(frame))))
}

/// The store's segments, mapped as rows name them, for a reader on a connection of its own (a SQL module's).
pub struct ObjectMaps(SegmentMaps);

impl ObjectMaps {
    /// The segments of the store at `database`.
    pub fn new(database: &Path) -> ObjectMaps {
        ObjectMaps(SegmentMaps::new(object_dir(database)))
    }
    /// A row's object (`codec`, `frame`, `seg`, `seg_at`, `seg_len`), as [`row_frame`].
    pub fn frame(&mut self, codec: i64, frame: Vec<u8>, seg: i64, at: i64, len: i64) -> StoreResult<Option<ObjectFrame>> {
        row_frame(&mut self.0, codec, frame, seg, at, len)
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
    tape: Arc<Vec<u8>>,
    tape_identity: HashKey,
    /// Whether the run's manifest names the source once written (`ContentStore::name_sources`).
    named: bool,
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
}

impl ObjectGather {
    fn add(&mut self, item: Item) {
        if self.at.contains_key(&item.key) {
            return;
        }
        self.bytes += item.source.len() + item.tape.len();
        self.at.insert(item.key, self.items.len());
        self.items.push(item);
    }
    fn held(item: &Item) -> Held {
        Arc::new(QueuedTape(ObjectBytes::Heap(Arc::clone(&item.tape))))
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
        let lens = self.items.iter().flat_map(|i| [i.source.len(), i.tape.len()]);
        let mut w = SegmentWriter::new(&self.dir, n, rerail_blocks::journal::reserve_for(n, lens, ALIGN));
        let mut placed = true;
        for (i, item) in self.items.iter().enumerate() {
            let key = i as i64;
            let digest = item.canonical.sha256_bytes().unwrap_or_default();
            let (source, tape) = (item.source.as_bytes(), &item.tape[..]);
            if w.put(source.len(), key, &digest, |out| out.copy_from_slice(source)).is_err() || w.put(tape.len(), key, &digest, |out| out.copy_from_slice(tape)).is_err() {
                placed = false;
                break;
            }
        }
        let segments = w.seal();
        let what = "tapes and their sources";
        if !placed {
            // No room for a segment (the disk): each file compressed and written in its own rows, as before.
            drop(segments);
            let keys = self.items.iter().map(|i| (i.key, Some(Self::held(i)))).collect();
            let bytes = self.bytes;
            return Some(Closed { what, bytes, keys, syms: Syms::None, job: ObjectsWrite::Each(self.items).work() });
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
                tape_len: i.tape.len(),
                tape_identity: i.tape_identity,
                named: i.named,
            })
            .collect();
        // The queue holds the rows; the bytes are the OS's pages.
        let bytes = rows.len() * 96;
        Some(Closed { what, bytes, keys, syms: Syms::None, job: ObjectsWrite::Placed { rows, segments }.work() })
    }
}

/// Sources and tapes as the writer writes them (decision 0306 §8).
pub(super) enum ObjectsWrite {
    /// A gather's, in its segments.
    Placed { rows: Vec<Row>, segments: SealedSegments },
    /// A gather's where no segment could be placed: each compressed into its own rows.
    Each(Vec<Item>),
    /// One file's, queued outside a run's waves.
    Lone { canonical: HashKey, grammar: (i64, i64, i64), frames: Arc<super::SourceAndTapeFrames>, named: bool },
}

impl ObjectsWrite {
    fn work(self) -> Work {
        Work(Kind::Objects(Box::new(self)))
    }

    pub(super) fn run(&mut self, store: &ContentStore) -> StoreResult<()> {
        match self {
            ObjectsWrite::Placed { rows, segments } => {
                let ids = store.put_objects_placed(rows, segments)?;
                store.name_sources(rows.iter().zip(ids).filter(|(row, _)| row.named).map(|(_, id)| id));
            }
            ObjectsWrite::Each(items) => {
                for i in items.iter() {
                    let (id, _) = store.put_source_and_tape(i.source.as_bytes(), i.canonical, i.grammar_id, i.grammar_version, i.tape_format_version, &i.tape)?;
                    if i.named {
                        store.name_sources([id]);
                    }
                }
            }
            ObjectsWrite::Lone { canonical, grammar: (gid, gv, fv), frames, named } => {
                let (id, _) = store.put_source_and_tape_framed(*canonical, *gid, *gv, *fv, frames)?;
                if *named {
                    store.name_sources([id]);
                }
            }
        }
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
    pub fn queue_source_and_tape(&self, key: Key, canonical: HashKey, grammar_id: i64, grammar_version: i64, tape_format_version: i64, source: &Arc<String>, tape: Arc<Vec<u8>>, named: bool) -> StoreResult<()> {
        let tape_identity = HashKey::tape_identity(canonical, &tape);
        let mut item = Some(Item { key, canonical, grammar_id, grammar_version, tape_format_version, source: Arc::clone(source), tape, tape_identity, named });
        let dir = object_dir(&self.database_path);
        let make = || ObjectGather { dir, items: Vec::new(), at: HashMap::new(), bytes: 0 };
        if self.gather("tapes and their sources", make, |g| g.add(item.take().expect("the file"))) {
            return Ok(());
        }
        let item = item.take().expect("the file, not gathered");
        let frames = Arc::new(self.source_and_tape_frames(item.source.as_bytes(), canonical, &item.tape)?);
        let write = ObjectsWrite::Lone { canonical, grammar: (grammar_id, grammar_version, tape_format_version), frames: Arc::clone(&frames), named: item.named };
        self.write_back_unless_full(Syms::None, "a tape and its source", frames.bytes(), vec![(key, Some(frames as Held))], write.work());
        Ok(())
    }

    /// The tape this process queued under `key` and the store has not written yet: gathered, placed or compressed.
    pub fn queued_tape(&self, key: &Key) -> Option<StoreResult<ObjectBytes>> {
        if let Some(q) = self.pending::<QueuedTape>(key) {
            return Some(Ok(q.0.clone()));
        }
        self.pending::<super::SourceAndTapeFrames>(key).map(|f| f.tape_bytes().map(|b| ObjectBytes::Heap(Arc::new(b))))
    }

    /// Writes a closed gather's files: its segments written back and numbered, each source and tape a row pointing at
    /// its entry (one already stored keeps its row), in one transaction; the segments named once it commits. Run
    /// again whole after a busy store: written back once, numbered again. The sources' ids, in `rows`' order.
    fn put_objects_placed(&self, rows: &[Row], segments: &mut SealedSegments) -> StoreResult<Vec<SourceId>> {
        self.require_write_capacity()?;
        segments.write_back()?;
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
                put_tape_placed_tx(t, source, tape, row.tape_len, row.tape_identity, place(2 * i + 1), created)?;
                ids.push(source);
            }
            Ok((ids, numbers))
        })?;
        segments.name(&object_dir(&self.database_path), &numbers)?;
        Ok(ids)
    }

    /// The tape and source segments' bytes, and the bytes their rows name: what compaction would leave.
    pub fn object_segment_bytes(&self) -> StoreResult<(u64, u64)> {
        let usage = self.object_segment_use()?;
        Ok((usage.held, usage.named))
    }

    /// The tape and source segments' bytes, the bytes their rows name, and their numbers.
    fn object_segment_use(&self) -> StoreResult<super::segments::SegmentUse> {
        let dir = object_dir(&self.database_path);
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(object_use(&connection, &object_kind(&dir))?)
    }

    /// Merges the tape and source segments compaction retires (`segments::retired`): every one where the bytes no row
    /// names (objects stored again, collected) are more than half of them and at least `floor`, else, past
    /// `segments::SEGMENT_BUDGET` of them, all but the oldest half of it; one write transaction. A row whose segment
    /// is not there is marked so (`seg` -1, never a segment's number: no later segment is read as its), absent to
    /// readers until its file is parsed and stored again; other rows name it, so it stays. Returns the bytes freed. A
    /// reader that mapped an old segment reads it on.
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

/// The tape and source segments, as compaction finds and merges them.
fn object_kind(dir: &Path) -> super::segments::SegmentKind<'_> {
    super::segments::SegmentKind { table: "object_segments", column: "seg", dir }
}

/// The tape and source segments' use, read in `t` (`segments::SegmentUse`).
fn object_use(t: &Connection, kind: &super::segments::SegmentKind<'_>) -> rusqlite::Result<super::segments::SegmentUse> {
    kind.usage(t, "SELECT (SELECT coalesce(sum(seg_len), 0) FROM source_objects WHERE seg > 0) + (SELECT coalesce(sum(seg_len), 0) FROM tape_objects WHERE seg > 0)")
}

/// A placed object's row, as compaction moves it: its table, the table's id column, and its id.
type ObjectRow = (&'static str, &'static str, i64);

/// The tapes' and sources' tables, each with its id column.
const OBJECT_TABLES: [(&str, &str); 2] = [("tape_objects", "tape_id"), ("source_objects", "source_id")];

/// The tapes and sources placed in segments numbered `from` and after, each table's in their order there.
fn placed_objects(t: &Connection, from: i64) -> rusqlite::Result<Vec<super::segments::Moved<ObjectRow>>> {
    let mut out = Vec::new();
    for (table, id) in OBJECT_TABLES {
        let mut st = t.prepare(&format!("SELECT {id}, seg, seg_at, seg_len FROM {table} WHERE seg > 0 AND seg >= ?1 ORDER BY seg, seg_at"))?;
        for row in st.query_map([from], |r| <(i64, i64, i64, i64)>::try_from(r))? {
            let (rowid, seg, at, len) = row?;
            out.push(super::segments::Moved { key: (table, id, rowid), doc: rowid, digest: [0; 32], place: Place { seg, at: at as u64, len: len as usize } });
        }
    }
    Ok(out)
}

/// A placed object whose segment is not there, marked so (`compact_object_segments`).
fn lose_object(t: &Connection, (table, id, rowid): &ObjectRow) -> rusqlite::Result<()> {
    t.execute(&format!("UPDATE {table} SET seg = -1 WHERE {id} = ?1"), [rowid]).map(drop)
}

/// A placed object's bytes, moved to segment `seg` at `at`.
fn move_object(t: &Connection, (table, id, rowid): &ObjectRow, seg: i64, at: i64) -> rusqlite::Result<()> {
    t.execute(&format!("UPDATE {table} SET seg = ?2, seg_at = ?3 WHERE {id} = ?1"), params![rowid, seg, at]).map(drop)
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
            store.queue_source_and_tape(key(i), HashKey::sha256(source.as_bytes()), 7, 1, 1, source, Arc::clone(tape), true).unwrap();
            match i {
                0 => assert!(matches!(store.queued_tape(&key(0)), Some(Ok(ObjectBytes::Heap(_))))),
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
