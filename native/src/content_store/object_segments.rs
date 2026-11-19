//! Tapes and sources in segments (decision 0295 step 5): a run's parsed files, their sources and tapes uncompressed,
//! placed in segments of `<store>.objects/` and read there in place, mapped; their rows (`source_objects`,
//! `tape_objects`) point at them (`seg`, `seg_at`, `seg_len`, codec [`CODEC_RAW`], no frame). A row of `seg` 0 holds
//! its bytes inline, zstd-compressed (`frame`): written outside a run's waves, or before.
//!
//! The run's parse workers add each file's source and tape to the run's gather ([`ObjectGather`], `write_back::Gather`),
//! held as the run holds them (no copy); once it holds the gather's cap, or the run's wave is sealed, it is placed in
//! one segment and queued as one job, its tapes read there meanwhile ([`QueuedTape`]).

use super::rows::{generation, intern_hash};
use super::segments::{ALIGN, Place, SealedSegments, SegmentBytes, SegmentMaps, SegmentWriter, segment_path};
use super::write_back::{Closed, Gather, Held, Key, Kind, Syms, Work};
use super::{CODEC_ZSTD, ContentStore, HashKey, SourceId, StoreError, StoreResult, TapeId, TapePayload};
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
    /// Decompressed from its row's frame into a buffer at 16 bytes, of its exact length: a load into the bytes its
    /// readers read (decision 0464: an image opens over it as over a mapping).
    Loaded(Arc<Loaded>),
    Mapped(SegmentBytes),
    /// A tape queued for the writer, in its home (decision 0353 step 4a: a slab's room, read there).
    Tape(TapeBytes),
}

impl std::ops::Deref for ObjectBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ObjectBytes::Heap(b) => b,
            ObjectBytes::Loaded(b) => (**b).as_ref(),
            ObjectBytes::Mapped(b) => b.bytes(),
            ObjectBytes::Tape(b) => (**b).as_ref(),
        }
    }
}

/// Bytes loaded at 16 bytes (`ObjectBytes::Loaded`).
pub struct Loaded {
    buf: Vec<u128>,
    len: usize,
}

impl Loaded {
    /// `frame` decompressed into a buffer of `len` bytes at 16: the frame must hold exactly that many.
    fn of_frame(frame: &[u8], len: usize) -> std::io::Result<Loaded> {
        let mut buf = vec![0u128; len.div_ceil(16)];
        // SAFETY: `buf` holds at least `len` bytes, borrowed here alone.
        let out = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), len) };
        super::compression::decompress_into(frame, out)?;
        Ok(Loaded { buf, len })
    }
}

impl AsRef<[u8]> for Loaded {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: `buf` holds `len` initialized bytes.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.len) }
    }
}

impl ObjectBytes {
    /// Its bytes as a vector of its own: a decompressed one as it is, a mapped one copied.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            ObjectBytes::Heap(b) => Arc::try_unwrap(b).unwrap_or_else(|b| (*b).clone()),
            ObjectBytes::Loaded(b) => (*b).as_ref().to_vec(),
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
    /// Its bytes at 16 bytes: a frame loaded (`Loaded`), a segment's read in place (decision 0464).
    pub fn bytes_aligned(&self, raw_length: usize) -> std::io::Result<ObjectBytes> {
        match self {
            ObjectFrame::Zstd(f) => Loaded::of_frame(f, raw_length).map(|b| ObjectBytes::Loaded(Arc::new(b))),
            ObjectFrame::Raw(b) => Ok(ObjectBytes::Mapped(b.clone())),
        }
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
    tape: TapeBytes,
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
                tape_len: (*i.tape).as_ref().len(),
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
                    let (id, _) = store.put_source_and_tape(i.source.as_bytes(), i.canonical, i.grammar_id, i.grammar_version, i.tape_format_version, (*i.tape).as_ref())?;
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
    pub fn queue_source_and_tape(&self, key: Key, canonical: HashKey, grammar_id: i64, grammar_version: i64, tape_format_version: i64, source: &Arc<String>, tape: TapeBytes, named: bool) -> StoreResult<()> {
        let tape_identity = HashKey::tape_identity(canonical, (*tape).as_ref());
        let mut item = Some(Item { key, canonical, grammar_id, grammar_version, tape_format_version, source: Arc::clone(source), tape, tape_identity, named });
        let dir = object_dir(&self.database_path);
        let make = || ObjectGather { dir, items: Vec::new(), at: HashMap::new(), bytes: 0 };
        if self.gather("tapes and their sources", make, |g| g.add(item.take().expect("the file"))) {
            return Ok(());
        }
        let item = item.take().expect("the file, not gathered");
        let frames = Arc::new(self.source_and_tape_frames(item.source.as_bytes(), canonical, (*item.tape).as_ref())?);
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

    /// Keeps `sources` (each its bytes, its sha256 and its aliases) placed in one new segment of the store's
    /// objects, uncompressed, to be read there in place (decision 0394): one write of the segment, one transaction
    /// for their rows, the segment named once it commits. A source the store holds inline is moved there; one it
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
            Some(super::segments::write_segments(&dir, &entries)?)
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

    /// Each of `hashes`' source, where the store holds it placed in a segment: its id, else `None` (inline, or not
    /// held).
    fn sources_placed(&self, hashes: &[HashKey]) -> StoreResult<Vec<Option<SourceId>>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached(
            "SELECT s.source_id FROM hashes h JOIN source_objects s ON s.canonical_hash_id = h.hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 AND s.seg > 0",
        )?;
        hashes.iter().map(|h| st.query_row(super::rows::hash_params(*h), |r| r.get::<_, i64>(0)).optional().map(|v| v.map(SourceId)).map_err(StoreError::from)).collect()
    }

    /// The source each of `aliases` names, in their order, in one read (decision 0394): one placed in a segment is
    /// read there, mapped (`ObjectBytes::in_place`): not decompressed, hashed or copied, its segment's check (when
    /// mapped) the check; one held inline is loaded at 16 bytes (`ObjectBytes::Loaded`, decision 0464) and checked
    /// against its hash, as `get_source` does. `None` where no source has the alias, or its segment is gone.
    pub fn sources_by_alias_in_place(&self, aliases: &[HashKey]) -> StoreResult<Vec<Option<ObjectBytes>>> {
        let rows: Vec<Option<(i64, i64, Vec<u8>, HashKey, [i64; 3])>> = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare_cached(
                "SELECT s.raw_length, s.codec_id, s.frame, c.algorithm_id, c.lane0, c.lane1, c.lane2, c.lane3, s.seg, s.seg_at, s.seg_len \
                 FROM hashes h JOIN source_aliases a ON a.alias_hash_id = h.hash_id JOIN source_objects s ON s.source_id = a.source_id \
                 JOIN hashes c ON c.hash_id = s.canonical_hash_id \
                 WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5",
            )?;
            aliases
                .iter()
                .map(|alias| {
                    st.query_row(super::rows::hash_params(*alias), |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, HashKey { algorithm: r.get(3)?, lanes: [r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?] }, [r.get(8)?, r.get(9)?, r.get(10)?]))
                    })
                    .optional()
                    .map_err(StoreError::from)
                })
                .collect::<StoreResult<_>>()?
        };
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Some((raw_length, codec, frame, key, [seg, seg_at, seg_len])) = row else {
                out.push(None);
                continue;
            };
            let length = usize::try_from(raw_length).map_err(|_| StoreError::Corrupt("invalid source length".into()))?;
            let Some(frame) = row_frame(&mut maps, codec, frame, seg, seg_at, seg_len)? else {
                out.push(None);
                continue;
            };
            let bytes = frame.bytes_aligned(length).map_err(|e| StoreError::Corrupt(format!("a source's frame: {e}")))?;
            if bytes.len() != length {
                return Err(StoreError::Corrupt("source length does not match metadata".to_owned()));
            }
            // Inline: its frame decompressed here, checked against the hash it is stored under.
            if !bytes.in_place() && key != HashKey::sha256(&bytes) {
                return Err(StoreError::Corrupt("source hash does not match metadata".to_owned()));
            }
            out.push(Some(bytes));
        }
        Ok(out)
    }

    /// The source `alias` names, read where the store holds it (`sources_by_alias_in_place`).
    pub fn source_by_alias_in_place(&self, alias: HashKey) -> StoreResult<Option<ObjectBytes>> {
        Ok(self.sources_by_alias_in_place(&[alias])?.pop().flatten())
    }

    /// The tape and source segments' bytes, and the bytes their rows name: what compaction would leave.
    pub fn object_segment_bytes(&self) -> StoreResult<(u64, u64)> {
        let (segs, named): (Vec<i64>, i64) = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let segs = connection.prepare("SELECT seg FROM object_segments")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            let named = connection.query_row(
                "SELECT (SELECT coalesce(sum(seg_len), 0) FROM source_objects WHERE seg > 0) + (SELECT coalesce(sum(seg_len), 0) FROM tape_objects WHERE seg > 0)",
                [],
                |r| r.get(0),
            )?;
            (segs, named)
        };
        let dir = object_dir(&self.database_path);
        let held = segs.iter().map(|&s| std::fs::metadata(segment_path(&dir, s)).map_or(0, |m| m.len())).sum();
        Ok((held, named as u64))
    }

    /// Copies every placed source's and tape's bytes to one new segment and removes the others, where the bytes no
    /// row names (objects stored again, collected) are more than half the segments and at least `floor`: one write
    /// transaction. A row whose segment is not there is marked so (`seg` -1, never a segment's number: no later
    /// segment is read as its), absent to readers until its file is parsed and stored again; other rows name it, so
    /// it stays. Returns the bytes freed. A reader that mapped an old segment reads it on.
    pub fn compact_object_segments(&self, floor: u64) -> StoreResult<u64> {
        let (held, named) = self.object_segment_bytes()?;
        if held < floor || held.saturating_sub(named) < held / 2 {
            return Ok(0);
        }
        let dir = object_dir(&self.database_path);
        let old = self.immediate_transaction_once(|t| {
            let segs: Vec<i64> = t.prepare("SELECT seg FROM object_segments")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            // (table, row id, place).
            let mut rows: Vec<(&str, i64, Place)> = Vec::new();
            for (table, id) in [("tape_objects", "tape_id"), ("source_objects", "source_id")] {
                let sql = format!("SELECT {id}, seg, seg_at, seg_len FROM {table} WHERE seg > 0 ORDER BY seg, seg_at");
                for r in t.prepare(&sql)?.query_map([], |r| Ok((r.get::<_, i64>(0)?, Place { seg: r.get(1)?, at: r.get::<_, i64>(2)? as u64, len: r.get::<_, i64>(3)? as usize })))? {
                    let (rowid, place) = r?;
                    rows.push((table, rowid, place));
                }
            }
            let mut maps = SegmentMaps::new(dir.clone());
            let mut kept: Vec<(&str, i64, SegmentBytes)> = Vec::new();
            for (table, rowid, place) in rows {
                match maps.at(place) {
                    Some(bytes) => kept.push((table, rowid, bytes)),
                    None => {
                        let id = if table == "tape_objects" { "tape_id" } else { "source_id" };
                        t.execute(&format!("UPDATE {table} SET seg = -1 WHERE {id} = ?1"), [rowid])?;
                    }
                }
            }
            t.execute("DELETE FROM object_segments", [])?;
            if kept.is_empty() {
                return Ok(segs);
            }
            let entries: Vec<(&[u8], i64, &[u8; 32])> = kept.iter().map(|(_, rowid, b)| (b.bytes(), *rowid, &[0u8; 32])).collect();
            let mut placed = super::segments::write_segments(&dir, &entries)?;
            if placed.files().len() != 1 {
                return Err(StoreError::Invalid("the objects' compaction made more than one segment".into()));
            }
            let seg = segs.iter().copied().max().unwrap_or(0) + 1;
            t.execute("INSERT INTO object_segments(seg) VALUES (?1)", [seg])?;
            for (i, (table, rowid, _)) in kept.iter().enumerate() {
                let id = if *table == "tape_objects" { "tape_id" } else { "source_id" };
                t.execute(&format!("UPDATE {table} SET seg = ?2, seg_at = ?3 WHERE {id} = ?1"), params![rowid, seg, placed.placed(i).1 as i64])?;
            }
            placed.name(&dir, &[seg])?;
            Ok(segs)
        })?;
        for seg in old {
            let _ = std::fs::remove_file(segment_path(&dir, seg));
        }
        let (now, _) = self.object_segment_bytes()?;
        Ok(held.saturating_sub(now))
    }
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

    /// Decision 0394: sources put placed are kept in one new segment, uncompressed, and read there by alias, mapped:
    /// not decompressed, hashed or copied. One the store held inline is moved there and keeps its id; one held placed
    /// keeps its place (no new segment); an alias of other bytes is refused. An inline source is still read
    /// decompressed and checked.
    #[test]
    fn sources_put_placed_are_read_in_place_by_alias() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let alias = |n: u8| HashKey::sha256(&[b"a test's alias ", &[n][..]].concat());
        let (old, new) = (vec![7u8; 5000], b"a run's rows".repeat(300));
        let old_id = store.put_source(&old, &[alias(0)]).unwrap();
        let read = |aliases: &[HashKey]| store.sources_by_alias_in_place(aliases).unwrap();
        let inline = read(&[alias(0)]).remove(0).expect("held");
        assert!(!inline.in_place() && *inline == old[..], "inline: decompressed and checked");
        let segs = |store: &ContentStore| store.connection.lock().unwrap().query_row("SELECT count(*) FROM object_segments", [], |r| r.get::<_, i64>(0)).unwrap();
        let ids = store.put_sources_placed(&[(&old, HashKey::sha256(&old), &[]), (&new, HashKey::sha256(&new), &[alias(1)])]).unwrap();
        assert_eq!(ids[0], old_id, "moved, its id kept");
        assert_eq!(segs(&store), 1, "one segment for both");
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
        assert_eq!(segs(&store), 1, "held placed: no new segment");
        // An alias of other bytes.
        assert!(store.put_sources_placed(&[(&old, HashKey::sha256(&old), &[alias(1)])]).is_err(), "an alias of other bytes is refused");
        // A new process reads them in place too.
        drop(store);
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        assert!(store.sources_by_alias_in_place(&[alias(0), alias(1)]).unwrap().iter().all(|b| b.as_ref().is_some_and(ObjectBytes::in_place)));
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
        let key = |i: usize| Key::of(&[b"a test's tape", &[i as u8]]);
        let queue = || {
            let _waves = write_back::run_waves();
            for (i, (source, tape)) in files.iter().enumerate() {
                store.queue_source_and_tape(key(i), HashKey::sha256(source.as_bytes()), 7, 1, 1, source, Arc::clone(tape) as TapeBytes, true).unwrap();
                match i {
                    // Gathered: read where the run holds it.
                    0 => assert!(matches!(store.queued_tape(&key(0)), Some(Ok(ObjectBytes::Tape(_))))),
                    // The gather full: placed, read in its segment, the wave not sealed yet.
                    3 => assert!(matches!(store.queued_tape(&key(0)), Some(Ok(ObjectBytes::Mapped(_))))),
                    _ => {}
                }
            }
        };
        queue();
        store.written().unwrap();
        let ids: Vec<SourceId> = files.iter().map(|(source, _)| store.source_by_hash(HashKey::sha256(source.as_bytes())).unwrap().expect("stored")).collect();
        assert_eq!(ids.len(), 4);
        let rows: Vec<(i64, i64, i64)> = store.connection.lock().unwrap().prepare("SELECT codec_id, seg, length(frame) FROM tape_objects UNION ALL SELECT codec_id, seg, length(frame) FROM source_objects").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
        assert!(rows.len() == 8 && rows.iter().all(|&(codec, seg, frame)| codec == CODEC_RAW && seg > 0 && frame == 0), "pointers, no frames: {rows:?}");
        for (id, (source, tape)) in ids.iter().zip(&files) {
            assert_eq!(tape_of(&store, *id).as_deref(), Some(&tape[..]));
            assert_eq!(store.get_source(*id).unwrap(), source.as_bytes());
        }
        let (held, named) = store.object_segment_bytes().unwrap();
        assert!(held >= named && named as usize > big, "{held} {named}");
        assert!(store.capacity().unwrap().bytes >= held, "the disk budget counts the segments");
        // A segment gone: a miss, until the file is stored again.
        for e in std::fs::read_dir(object_dir(&store.database_path)).unwrap() {
            std::fs::remove_file(e.unwrap().path()).unwrap();
        }
        assert_eq!(tape_of(&store, ids[0]), None);
        queue();
        store.written().unwrap();
        assert_eq!(tape_of(&store, ids[0]).as_deref(), Some(&files[0].1[..]));
        // Stored again: the first copy is no row's, and compaction keeps one segment.
        queue();
        store.written().unwrap();
        assert!(store.compact_object_segments(0).unwrap() > 0);
        assert_eq!(std::fs::read_dir(object_dir(&store.database_path)).unwrap().count(), 1, "one segment left");
        for (id, (source, tape)) in ids.iter().zip(&files) {
            assert_eq!(tape_of(&store, *id).as_deref(), Some(&tape[..]));
            assert_eq!(store.get_source(*id).unwrap(), source.as_bytes());
        }
    }
}
