//! Facts as stored: every relation of every fact space, as frames (decision 0104; docs/final-shape/notes/t33 K1).
//!
//! A frame is the rows of one relation of one fact space, or of one partition of it (a document, a granule; `0` is
//! the whole relation), each column as zigzag varints of its differences (`cross_rows::encode`). It is keyed by:
//!
//! - **space**: the fact space, a digest of the corpus root and the space's name (a phase, a program's cross part);
//! - **rel** and **ident**: the relation's name, and its identity. The identity is the digest of the rules it is
//!   derived from, their reads' identities, and the engine (`plan::identities`), so a rule edit makes another
//!   relation, which is derived again. A relation that depends on no rule (a native answer's) has its own;
//! - **part**: the partition.
//!
//! A derived frame records what it was derived from: each relation its stratum read, with the digest that read had
//! (03 §1, "the versions of the facts it was derived from", at the grain of a relation). It is current while every
//! read has that digest still. That is what lets relations of the same identity be one copy for every program that
//! has them, in any order of runs.
//!
//! Frames are written only by the store's writer (decision 0102), behind the run. What a space keeps is what the kept
//! manifests reach (decision 0227, `collect_unreachable_frames`): a memo entry while its plan is live and a kept
//! manifest names its content, a maintained frame while a live program (a part's key, named by a kept manifest)
//! derives its relation there. No run marks what it used: a run that changes nothing writes nothing.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::{ContentStore, StoreResult};
use rusqlite::{Connection, OptionalExtension as _, params};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(super) mod memo_pack;
pub use memo_pack::{MemoBlob, MemoPut, MemoRow};
pub(super) use memo_pack::drop_memo_space;

/// The run each space last counted in this process, by its token: a later program of that run puts under the same
/// stamp. A new process counts its first run of a space, since its tokens are new.
fn counted() -> &'static std::sync::Mutex<HashMap<[u8; 32], u64>> {
    static COUNTED: std::sync::OnceLock<std::sync::Mutex<HashMap<[u8; 32], u64>>> = std::sync::OnceLock::new();
    COUNTED.get_or_init(Default::default)
}

/// What a program derives in a space, with what it puts there: the relations, the run's token (`0`: a write of its
/// own, counted each time) and the program (its key). Its relations are kept while the program is live
/// (`collect_unreachable_frames`).
pub type Used = (Vec<Rel>, u64, [u8; 32]);

// The frames' tables (`frames`, `frame_part_used`, `frame_spaces`, `frame_confirmed`, `frame_program_used`) are
// the store's schema (`schema::SCHEMA`): every store has them, so nothing here checks for them or makes them.

/// A relation's key in a run's use of its space by program (`frame_program_used`).
fn used_key(rel: &Rel) -> u64 {
    confirm_key(rel, i64::MIN)
}

fn encode_keys(keys: &[u64]) -> Vec<u8> {
    keys.iter().flat_map(|k| k.to_le_bytes()).collect()
}

fn decode_keys(data: &[u8]) -> Vec<u64> {
    data.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes"))).collect()
}

/// A relation as its frame names it: its name and its identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rel {
    pub name: String,
    pub ident: [u8; 32],
}

/// A frame's rows: each column's value type (the engine's codes), and the rows, flat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rows {
    pub types: Vec<u8>,
    pub rows: Vec<i64>,
    /// The rows' count (a relation of no columns has rows but no values).
    pub n: usize,
}

/// A stored frame, without its rows: its digest and what it was derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub digest: u64,
    /// Each relation it was derived from, by name, with that relation's digest then.
    pub reads: Vec<(String, u64)>,
}

/// A frame whose rows stay as stored and whose reads are told again: the relation's rows did not change, and what
/// it read did (decision 0117 step 4). Its digest stays.
#[derive(Clone, Debug)]
pub struct Retag {
    pub rel: Rel,
    pub part: i64,
    pub reads: Vec<(String, u64)>,
}

/// A relation's rows changed by a few (decision 0117 step 4): the rows gone and added since its frame, written after
/// it as a delta frame, with the relation's digest and reads now. Read back, its frame is its rows after its deltas.
#[derive(Clone, Debug)]
pub struct Delta {
    pub rel: Rel,
    pub digest: u64,
    pub reads: Vec<(String, u64)>,
    pub types: Vec<u8>,
    pub gone: Vec<i64>,
    pub added: Vec<i64>,
}

/// A frame a run writes.
pub struct Put {
    pub rel: Rel,
    pub part: i64,
    pub digest: u64,
    pub reads: Vec<(String, u64)>,
    pub rows: Rows,
}

/// One admitted fact relation. Durable rows remain encoded until a consumer needs them.
/// A queued frame is encoded as it is queued: the snapshot and the writer share it. This value belongs to the run, not a
/// separate cache.
pub struct Admitted {
    pub stored: Stored,
    data: Data,
}

/// A frame's encoded rows: in memory (made by the run and not yet placed, or read inline from a row), or where they
/// are in a segment, mapped (decision 0295 §3–§4: the run's queued frames, and the store's frames written since).
#[derive(Clone)]
enum Bytes {
    Heap(Arc<Vec<u8>>),
    Mapped(super::segments::SegmentBytes),
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Heap(b) => b,
            Bytes::Mapped(b) => b.bytes(),
        }
    }
}

/// The directory of the store's frame segments: beside its file.
fn frame_dir(database: &std::path::Path) -> std::path::PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".frames");
    std::path::PathBuf::from(name)
}

/// A frame's bytes as its row names them: inline (`seg` 0: the row's `data`), or a range of a segment, mapped. `None`
/// when that segment is not there or holds no such range: the frame is absent, derived again.
fn row_bytes(maps: &mut super::segments::SegmentMaps, data: Vec<u8>, seg: i64, at: i64, len: i64) -> Option<Bytes> {
    if seg == 0 {
        return Some(Bytes::Heap(Arc::new(data)));
    }
    maps.at(super::segments::Place { seg, at: at as u64, len: len as usize }).map(Bytes::Mapped)
}

#[derive(Clone)]
enum Data {
    /// `store`: the bytes are the store's, read from it, their symbols the store's ids (decision 0170): mapped to this
    /// process's as they are decoded. `None`: this process's own (encoded as queued, or its snapshot of what it wrote).
    Encoded { types: Vec<u8>, n: usize, bytes: Bytes, store: Option<Arc<super::atoms::Atoms>> },
    Metadata,
    /// A relation's rows as its base frame (part 0) and the deltas written after it, oldest first (decision 0117
    /// step 4): a run that changed a few rows of a large relation writes those, not the relation. `n`: the rows after
    /// the last; `delta_rows`: the deltas' rows.
    Chain { base: Box<Data>, deltas: Vec<Arc<Admitted>>, n: usize, delta_rows: usize },
}

/// The first type of a delta frame's (decision 0117 step 4): its first column is each row's sign (-1 gone, 1 added),
/// its others the relation's.
pub const DELTA: u8 = b'~';

/// A frame's part for the `k`th delta (from 1) after a relation's base frame.
fn delta_part(k: u32) -> i64 {
    -(k as i64)
}

/// The rows of a relation after `deltas` (delta frames, oldest first) applied to `rows`.
fn apply_deltas(mut rows: Rows, deltas: &[Arc<Admitted>]) -> StoreResult<Rows> {
    let w = rows.types.len();
    for d in deltas {
        let got = d.rows()?;
        if got.types.first() != Some(&DELTA) || got.types.len() != w + 1 || w == 0 {
            return Err(super::StoreError::Invalid("a delta frame of another shape".into()));
        }
        let mut gone: std::collections::HashSet<&[i64]> = std::collections::HashSet::new();
        let mut added: Vec<i64> = Vec::new();
        for row in got.rows.chunks_exact(w + 1).take(got.n) {
            if row[0] < 0 { gone.insert(&row[1..]); } else { added.extend_from_slice(&row[1..]); }
        }
        if !gone.is_empty() {
            let mut kept = Vec::with_capacity(rows.rows.len());
            let mut n = 0;
            for row in rows.rows.chunks_exact(w).take(rows.n) {
                if !gone.contains(row) {
                    kept.extend_from_slice(row);
                    n += 1;
                }
            }
            rows.rows = kept;
            rows.n = n;
        }
        rows.n += added.len() / w;
        rows.rows.extend(added);
    }
    Ok(rows)
}

fn rows_of(data: &Data) -> StoreResult<Rows> {
    match data {
        Data::Encoded { types, n, bytes, store } => {
            let width = types.len();
            let mut cols = vec![Vec::new(); width];
            super::cross_rows::decode(width, *n, bytes, &mut cols)?;
            let mut rows = Vec::with_capacity(width * n);
            for r in 0..if width == 0 { 0 } else { *n } {
                rows.extend(cols.iter().map(|c| c[r]));
            }
            if let Some(atoms) = store {
                super::atoms::map_columns(types, &mut rows, |v| atoms.to_local(super::StoreIds::as_stored(v)).map(drop))?;
            }
            Ok(Rows { types: types.clone(), rows, n: *n })
        }
        Data::Metadata => Err(super::StoreError::Invalid("frame payload was not admitted".into())),
        Data::Chain { base, deltas, .. } => apply_deltas(rows_of(base)?, deltas),
    }
}

impl Admitted {
    /// The deltas after its base frame (decision 0117 step 4): how many, and their rows.
    pub fn chain(&self) -> (u32, usize) {
        match &self.data {
            Data::Chain { deltas, delta_rows, .. } => (deltas.len() as u32, *delta_rows),
            _ => (0, 0),
        }
    }

    /// Its base frame and deltas, with `delta` (a delta frame) after them.
    fn with_delta(&self, stored: Stored, delta: Arc<Admitted>, gone: usize, added: usize) -> Admitted {
        let (base, mut deltas, n, delta_rows) = match &self.data {
            Data::Chain { base, deltas, n, delta_rows } => (base.clone(), deltas.clone(), *n, *delta_rows),
            other => (Box::new(other.clone()), Vec::new(), self.len().unwrap_or(0), 0),
        };
        deltas.push(delta);
        Admitted { stored, data: Data::Chain { base, deltas, n: (n + added).saturating_sub(gone), delta_rows: delta_rows + gone + added } }
    }

    pub fn rows(&self) -> StoreResult<Rows> {
        match &self.data {
            Data::Chain { .. } | Data::Encoded { .. } => rows_of(&self.data),
            Data::Metadata => Err(super::StoreError::Invalid("frame payload was not admitted".into())),
        }
    }

    /// Appends the store's ids its rows' symbol columns name, for rows read from the store (as the store numbers them).
    fn stored_symbols(&self, out: &mut Vec<i64>) -> StoreResult<()> {
        fn of(data: &Data, out: &mut Vec<i64>) -> StoreResult<()> {
            match data {
                Data::Encoded { types, n, bytes, store: Some(_) } => {
                    if !types.contains(&b's') {
                        return Ok(());
                    }
                    let mut cols = vec![Vec::new(); types.len()];
                    super::cross_rows::decode(types.len(), *n, bytes, &mut cols)?;
                    for (c, col) in cols.into_iter().enumerate() {
                        if types[c] == b's' {
                            out.extend(col);
                        }
                    }
                    Ok(())
                }
                Data::Chain { base, deltas, .. } => {
                    of(base, out)?;
                    deltas.iter().try_for_each(|d| of(&d.data, out))
                }
                Data::Encoded { store: None, .. } | Data::Metadata => Ok(()),
            }
        }
        of(&self.data, out)
    }

    /// The store whose ids its rows' symbols are (decision 0170): set for rows read from the store, none for this
    /// process's own. Rows `rows` gives map the typed (`b's'`) columns; a reader of untyped ones maps them itself.
    pub fn store_atoms(&self) -> Option<Arc<super::atoms::Atoms>> {
        fn of(data: &Data) -> Option<Arc<super::atoms::Atoms>> {
            match data {
                Data::Encoded { store, .. } => store.clone(),
                Data::Chain { base, deltas, .. } => of(base).or_else(|| deltas.iter().find_map(|d| of(&d.data))),
                Data::Metadata => None,
            }
        }
        of(&self.data)
    }

    /// Its rows' width (their columns), where its rows were admitted: known without decoding them.
    pub fn width(&self) -> Option<usize> {
        fn of(data: &Data) -> Option<usize> {
            match data {
                Data::Encoded { types, .. } => Some(types.len()),
                Data::Chain { base, .. } => of(base),
                Data::Metadata => None,
            }
        }
        of(&self.data)
    }

    /// Its rows' count, where its rows were admitted.
    pub fn len(&self) -> Option<usize> {
        match &self.data {
            Data::Encoded { n, .. } => Some(*n),
            Data::Chain { n, .. } => Some(*n),
            Data::Metadata => None,
        }
    }

    pub fn encoded_bytes(&self) -> usize {
        match &self.data {
            Data::Encoded { bytes, .. } => bytes.len(),
            Data::Chain { base, deltas, .. } => {
                let b = Admitted { stored: self.stored.clone(), data: (**base).clone() };
                b.encoded_bytes() + deltas.iter().map(|d| d.encoded_bytes()).sum::<usize>()
            }
            Data::Metadata => 0,
        }
    }
}

// Stay below even SQLite's older 999-parameter limit. Full batches share one prepared shape.
const READ_BATCH: usize = 128;

/// Every frame of a store as this process holds it (decision 0104: a run reads before its first wave): read once
/// (`preload_frames`), kept current by every write this process queues or makes, by space, then relation and part.
/// Shared by the store's handles in this process, by its path. With the frames, what else the store keeps of them:
/// each space's confirmations (`ContentStore::confirmed`), read with the frames and kept current by every write this
/// process queues, and the memo spaces whose entries were admitted (`admit_memo_entries`).
#[derive(Default)]
struct Snapshot {
    frames: HashMap<[u8; 32], HashMap<(Rel, i64), Arc<Admitted>>>,
    confirmed: Confirms,
    admitted: HashSet<[u8; 32]>,
    /// The memo entries admitted (`admit_memo_entries`) and written since, by space and part (decision 0240 §4).
    memo: HashMap<[u8; 32], HashMap<i64, Arc<MemoBlob>>>,
    /// The cross spaces a run read before its first wave, by identity (Q19: each by the run whose own it is).
    preloaded: HashSet<[u8; 32]>,
    /// A dev trace's (`RERAIL_FRAMES_TRACE`): the space read by identity, and its frames' identities left out.
    left_out: Option<([u8; 32], HashSet<Rel>)>,
    /// The (space, program) pairs known to hold there (`hold_frames`, decision 0302): each looked up once a process.
    holding: HashSet<([u8; 32], [u8; 32])>,
}

/// A root's cross part's space, as a run reads it before its first wave (`preload_frames_live`, Q19).
#[derive(Clone, Copy)]
pub struct CrossSpace<'a> {
    /// The root's cross space (the cross part of a run of no phase).
    pub space: [u8; 32],
    /// Whether it is this run's own (a run of no phase): only that run reads it.
    pub own: bool,
    /// The identities its programs can read there; `None`: every frame of it.
    pub rels: Option<&'a HashSet<Rel>>,
    /// A root's cross space (by its path, as `corpus_roots` records it): the store's first read leaves every root's
    /// out, each read by its own run, by identity; whichever run reads the store's frames first (phase 0's, on a
    /// rule edit: before the corpus run starts, a root of its own).
    pub cross_of: fn(&std::path::Path) -> [u8; 32],
}

impl std::ops::Deref for Snapshot {
    type Target = HashMap<[u8; 32], HashMap<(Rel, i64), Arc<Admitted>>>;
    fn deref(&self) -> &Self::Target {
        &self.frames
    }
}

impl std::ops::DerefMut for Snapshot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frames
    }
}

/// Each space's confirmations: by frame (`confirm_key`), the hash of the reads it was confirmed under.
type Confirms = HashMap<[u8; 32], HashMap<u64, u64>>;

fn snapshots() -> &'static std::sync::Mutex<HashMap<std::path::PathBuf, Arc<std::sync::Mutex<Snapshot>>>> {
    static SNAPSHOTS: std::sync::OnceLock<std::sync::Mutex<HashMap<std::path::PathBuf, Arc<std::sync::Mutex<Snapshot>>>>> = std::sync::OnceLock::new();
    SNAPSHOTS.get_or_init(Default::default)
}

/// The relations a corpus memo's space kept per file as frames before decision 0240 (`local`, its entries;
/// `local_out`, their outputs again): no longer written or read. Preloading skips them; collection forgets them.
const MEMO_RELS: [&str; 2] = ["local", "local_out"];


/// Whether `frame` is a delta frame (decision 0117 step 4).
fn is_delta(part: i64, frame: &Admitted) -> bool {
    part < 0 && matches!(&frame.data, Data::Encoded { types, .. } if types.first() == Some(&DELTA))
}

/// Each relation's base frame in `held` with the `k` delta frames its header names after it (oldest first); every
/// delta frame is taken out of `held`.
fn compose_chains(held: &mut HashMap<(Rel, i64), Arc<Admitted>>, chains: Option<&HashMap<Rel, u32>>) -> StoreResult<()> {
    let named = chains;
    let mut chains: HashMap<Rel, Vec<(i64, Arc<Admitted>)>> = HashMap::new();
    let keys: Vec<(Rel, i64)> = held.iter().filter(|((_, p), f)| is_delta(*p, f)).map(|(k, _)| k.clone()).collect();
    for key in keys {
        let frame = held.remove(&key).expect("a delta frame");
        chains.entry(key.0).or_default().push((key.1, frame));
    }
    for (rel, mut deltas) in chains {
        let Some(base) = held.get(&(rel.clone(), 0)).cloned() else { continue };
        let k = named.and_then(|c| c.get(&rel)).copied().unwrap_or(0);
        deltas.retain(|(p, _)| *p >= delta_part(k));
        if deltas.is_empty() {
            continue;
        }
        deltas.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
        let mut whole = (*base).clone_frame();
        for (_, d) in deltas {
            let rows = d.rows()?;
            let gone = rows.rows.chunks_exact(rows.types.len().max(1)).take(rows.n).filter(|r| r[0] < 0).count();
            whole = whole.with_delta(base.stored.clone(), d, gone, rows.n - gone);
        }
        held.insert((rel, 0), Arc::new(whole));
    }
    Ok(())
}

impl Admitted {
    fn clone_frame(&self) -> Admitted {
        Admitted { stored: self.stored.clone(), data: self.data.clone() }
    }
}

fn frame_key(space: &[u8; 32], rel: &Rel, part: i64) -> super::write_back::Key {
    super::write_back::Key::of(&[b"frame", space, rel.name.as_bytes(), &rel.ident, &part.to_le_bytes()])
}

/// A space's frames as the writer writes them (`queue_frames`, decision 0306 §8): placed in segments where they fit.
pub(super) struct FramesWrite {
    space: [u8; 32],
    frames: Vec<Encoded>,
    delta_frames: Vec<Encoded>,
    retags: Vec<Retag>,
    heads: Vec<(Rel, u64, Vec<(String, u64)>)>,
    gone: Vec<(Rel, i64)>,
    used: Option<Used>,
    segments: Option<super::segments::SealedSegments>,
}

impl FramesWrite {
    pub(super) fn run(&mut self, store: &ContentStore) -> StoreResult<()> {
        // Attempted once: a busy store leaves these frames unwritten, and the next run derives them again.
        let rows: Vec<Framed> = self.frames.iter().chain(&self.delta_frames).filter_map(Encoded::framed).collect();
        let used = self.used.as_ref().map(|(rels, run, program)| (rels.as_slice(), *run, *program));
        let _ = store.write_frames_placed(&self.space, &rows, self.delta_frames.len(), &self.retags, &self.heads, &self.gone, used, self.segments.as_mut());
        Ok(())
    }
}

impl ContentStore {
    /// The snapshot of this store's file, by its canonical path: handles that name it differently share it.
    fn snapshot(&self) -> Option<Arc<std::sync::Mutex<Snapshot>>> {
        snapshots().lock().expect("the frames' snapshots").get(&super::atoms::key(&self.database_path)).cloned()
    }

    /// Reads every frame of the store, once a process (decision 0104: before a run's first wave): from then on the
    /// frames are read from memory, and every write of this process updates it. Returns the frames and their
    /// encoded bytes read now (none when an earlier run of the process read them).
    pub fn preload_frames(&self) -> StoreResult<(usize, usize)> {
        self.preload_frames_live(None)
    }

    /// The frames a run reads before its first wave (decision 0104, Q19), the first run of a process installing the
    /// snapshot. The memo's entries are not frames (decision 0240 §4); a run admits its plan's
    /// (`admit_memo_entries`).
    ///
    /// Of `cross` (the root's cross part's space), when it is the run's own and no run of the process read it: the
    /// frames of the identities its programs can read (every part of each), not every kept run's (an earlier build's,
    /// an earlier edit's: a relation's identity is of the engine and its rules, `plan::identities`); without them, all
    /// of it. A run of a phase does not read it. A frame of it outside the set is absent to the process, as one never
    /// kept is: its relation is derived again. The snapshot's install leaves every recorded root's cross space out
    /// (`CrossSpace::cross_of`), whichever run installs it; every other space is read whole.
    pub fn preload_frames_live(&self, cross: Option<CrossSpace<'_>>) -> StoreResult<(usize, usize)> {
        let (mut frames, mut bytes) = match self.snapshot() {
            Some(_) => (0, 0),
            None => self.install_snapshot(cross.as_ref())?,
        };
        let snapshot = self.snapshot().expect("the snapshot, installed");
        if let Some(c) = cross.filter(|c| c.own)
            && snapshot.lock().expect("the frames' snapshot").preloaded.insert(c.space)
        {
            let (got, left) = self.read_cross_space(&c)?;
            frames += got.len();
            bytes += got.values().map(|f| f.encoded_bytes()).sum::<usize>();
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            if let Some(left) = left {
                snapshot.left_out = Some((c.space, left));
            }
            let held = snapshot.frames.entry(c.space).or_default();
            // What this process wrote since is newer, and stays.
            for (k, f) in got {
                held.entry(k).or_insert(f);
            }
        }
        Ok((frames, bytes))
    }

    /// The snapshot, read: every frame but the memo's of before decision 0240 (`MEMO_RELS`), which nothing reads,
    /// and, with `cross`, every recorded root's cross space (`CrossSpace::cross_of`) and the run's own.
    fn install_snapshot(&self, cross: Option<&CrossSpace<'_>>) -> StoreResult<(usize, usize)> {
        let _span = rerail_prof::code_rule_dsl::logic::profile::span(|| "the store's frames, read once a process".into());
        let mut snapshot = Snapshot::default();
        let (mut frames, mut bytes) = (0, 0);
        {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut skip: Vec<[u8; 32]> = Vec::new();
            if let Some(c) = cross {
                let roots: Vec<String> = connection.prepare("SELECT path FROM corpus_roots")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                skip.extend(roots.iter().map(|r| (c.cross_of)(std::path::Path::new(r))));
                skip.push(c.space);
            }
            let skip: rusqlite::vtab::array::Array = std::rc::Rc::new(skip.iter().map(|s| rusqlite::types::Value::from(s.to_vec())).collect());
            let mut st = connection.prepare("SELECT space, rel, ident, part, digest, reads, rows, types, data, seg, seg_at, seg_len FROM frames WHERE rel NOT IN (?1, ?2) AND space NOT IN rarray(?3)")?;
            let mut maps = super::segments::SegmentMaps::new(frame_dir(&self.database_path));
            let mut chains: HashMap<[u8; 32], HashMap<Rel, u32>> = HashMap::new();
            let rows = st.query_map(params![MEMO_RELS[0], MEMO_RELS[1], skip], |row| {
                let (space, ident): (Vec<u8>, Vec<u8>) = (row.get(0)?, row.get(2)?);
                let space: [u8; 32] = space.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let ident: [u8; 32] = ident.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let Some(bytes) = row_bytes(&mut maps, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?) else { return Ok(None) };
                let (reads, k) = decode_reads_chain(&row.get::<_, Vec<u8>>(5)?);
                Ok(Some((space, (Rel { name: row.get(1)?, ident }, row.get::<_, i64>(3)?), k, Arc::new(Admitted {
                    stored: Stored { digest: row.get::<_, i64>(4)? as u64, reads },
                    data: Data::Encoded { n: row.get::<_, i64>(6)? as usize, types: row.get(7)?, bytes, store: Some(self.atoms_handle()) },
                }))))
            })?;
            let mut read: Vec<Arc<Admitted>> = Vec::new();
            for row in rows {
                let Some((space, key, k, frame)) = row? else { continue };
                frames += 1;
                bytes += frame.encoded_bytes();
                read.push(Arc::clone(&frame));
                if k > 0 && key.1 == 0 {
                    chains.entry(space).or_default().insert(key.0.clone(), k);
                }
                snapshot.entry(space).or_default().insert(key, frame);
            }
            drop(st);
            let mut confirmed = Confirms::new();
            {
                let mut st = connection.prepare("SELECT space, data FROM frame_confirmed")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
                for row in rows {
                    let (space, data) = row?;
                    if let Ok(space) = <[u8; 32]>::try_from(space.as_slice()) {
                        confirmed.insert(space, decode_confirmed(&data));
                    }
                }
            }
            snapshot.confirmed = confirmed;
            // Which programs hold which spaces (`hold_frames`, decision 0302): a few rows, read here, not in a wave.
            {
                let mut st = connection.prepare("SELECT space, program FROM frame_program_used WHERE run = 0")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
                for row in rows {
                    let (space, program) = row?;
                    if let (Ok(space), Ok(program)) = (<[u8; 32]>::try_from(space.as_slice()), <[u8; 32]>::try_from(program.as_slice())) {
                        snapshot.holding.insert((space, program));
                    }
                }
            }
            drop(connection);
            let _held = rerail_prof::code_rule_dsl::logic::profile::span(|| "their symbols held".into());
            hold_symbols(&self.atoms, read.iter())?;
            drop(_held);
            let _chains = rerail_prof::code_rule_dsl::logic::profile::span(|| "their delta chains composed".into());
            for (space, held) in snapshot.iter_mut() {
                compose_chains(held, chains.get(space))?;
            }
        }
        snapshots().lock().expect("the frames' snapshots").entry(super::atoms::key(&self.database_path)).or_insert_with(|| Arc::new(std::sync::Mutex::new(snapshot)));
        Ok((frames, bytes))
    }

    /// `cross`'s frames (`preload_frames_live`): those of its identities, every part; all of them when it names none.
    /// One scan of the space in its key's order, the frames of other identities let go as they are read: a seek for
    /// each identity reads the file's pages at random (1.5 s against 0.7 s, ep2). With a dev trace
    /// (`RERAIL_FRAMES_TRACE`), the identities of the space's frames left out, so a read of one names a reader the set
    /// missed (`read_frame_parts_from`).
    fn read_cross_space(&self, cross: &CrossSpace<'_>) -> StoreResult<(HashMap<(Rel, i64), Arc<Admitted>>, Option<HashSet<Rel>>)> {
        let _span = rerail_prof::code_rule_dsl::logic::profile::span(|| "the cross part's space, by identity (Q19)".into());
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let space = cross.space;
        let atoms = self.atoms_handle();
        let mut got: HashMap<(Rel, i64), Arc<Admitted>> = HashMap::new();
        let mut chains: HashMap<Rel, u32> = HashMap::new();
        {
            let mut st = connection.prepare("SELECT rel, ident, part, digest, reads, rows, types, data, seg, seg_at, seg_len FROM frames WHERE space = ?1")?;
            let mut maps = super::segments::SegmentMaps::new(frame_dir(&self.database_path));
            let mut rows = st.query([&space[..]])?;
            while let Some(row) = rows.next()? {
                let ident: [u8; 32] = row.get::<_, Vec<u8>>(1)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let rel = Rel { name: row.get(0)?, ident };
                if cross.rels.is_some_and(|k| !k.contains(&rel)) {
                    continue;
                }
                let part: i64 = row.get(2)?;
                let (reads, k) = decode_reads_chain(&row.get::<_, Vec<u8>>(4)?);
                if k > 0 && part == 0 {
                    chains.insert(rel.clone(), k);
                }
                let Some(bytes) = row_bytes(&mut maps, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?) else { continue };
                got.insert((rel, part), Arc::new(Admitted {
                    stored: Stored { digest: row.get::<_, i64>(3)? as u64, reads },
                    data: Data::Encoded { n: row.get::<_, i64>(5)? as usize, types: row.get(6)?, bytes, store: Some(Arc::clone(&atoms)) },
                }));
            }
        }
        let left = match cross.rels.filter(|_| std::env::var_os("RERAIL_FRAMES_TRACE").is_some()) {
            Some(rels) => {
                let mut st = connection.prepare("SELECT DISTINCT rel, ident FROM frames WHERE space = ?1")?;
                let keys = st.query_map([&space[..]], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
                let mut left: HashSet<Rel> = HashSet::new();
                for key in keys {
                    let (name, ident) = key?;
                    if let Ok(ident) = <[u8; 32]>::try_from(ident.as_slice()) {
                        let rel = Rel { name, ident };
                        if !rels.contains(&rel) {
                            left.insert(rel);
                        }
                    }
                }
                eprintln!("RERAIL_FRAMES_TRACE preload by identity: {} identities asked, {} frames read, {} identities of the space left out", rels.len(), got.len(), left.len());
                Some(left)
            }
            None => None,
        };
        drop(connection);
        let _held = rerail_prof::code_rule_dsl::logic::profile::span(|| "their symbols held".into());
        hold_symbols(&self.atoms, got.values())?;
        compose_chains(&mut got, Some(&chains))?;
        Ok((got, left))
    }

    /// The stored frames of `rels` in `space`, partition `part`, without their rows.
    /// The space's confirmations (decision 0117 step 5): each frame whose rows were found unchanged by a run that
    /// read other digests than its stored reads name, with the hash of the reads it was confirmed under
    /// (`reads_hash`). A frame is as current as its stored reads say, or as a confirmation says. Queued
    /// confirmations are the snapshot's frames' reads (updated as they are queued), so the store's row serves.
    pub fn confirmed(&self, space: &[u8; 32]) -> StoreResult<HashMap<u64, u64>> {
        // With the frames read before the first wave, so are the confirmations: no SQL inside the waves.
        if let Some(snapshot) = self.snapshot() {
            return Ok(snapshot.lock().expect("the frames' snapshot").confirmed.get(space).cloned().unwrap_or_default());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let row: Option<Vec<u8>> = connection.query_row_cached("SELECT data FROM frame_confirmed WHERE space = ?1", [&space[..]], |r| r.get(0)).optional()?;
        Ok(row.as_deref().map(decode_confirmed).unwrap_or_default())
    }

    pub fn frames(&self, space: &[u8; 32], rels: &[Rel], part: i64) -> StoreResult<HashMap<Rel, Stored>> {
        let keys: Vec<_> = rels.iter().cloned().map(|r| (r, part)).collect();
        Ok(self.read_frame_parts(space, &keys, false)?.into_iter().map(|((r, _), f)| (r, f.stored.clone())).collect())
    }

    /// Admit requested relations before their consumers run. No consumer needs another store read.
    pub fn admit_frames(&self, space: &[u8; 32], rels: &[Rel], part: i64) -> StoreResult<HashMap<Rel, Arc<Admitted>>> {
        let keys: Vec<_> = rels.iter().cloned().map(|r| (r, part)).collect();
        Ok(self.admit_frame_parts(space, &keys)?.into_iter().map(|((r, _), f)| (r, f)).collect())
    }

    /// Admit a demand set across relation partitions. Granules share the same full-key probes as whole relations.
    pub fn admit_frame_parts(&self, space: &[u8; 32], keys: &[(Rel, i64)]) -> StoreResult<HashMap<(Rel, i64), Arc<Admitted>>> {
        self.read_frame_parts(space, keys, true)
    }

    fn read_frame_parts(&self, space: &[u8; 32], keys: &[(Rel, i64)], payload: bool) -> StoreResult<HashMap<(Rel, i64), Arc<Admitted>>> {
        self.read_frame_parts_from(space, keys, payload, true)
    }

    /// `read_frame_parts`, from the store's file past the snapshot unless `snapshot` (queued writes first, always).
    fn read_frame_parts_from(&self, space: &[u8; 32], keys: &[(Rel, i64)], payload: bool, snapshot: bool) -> StoreResult<HashMap<(Rel, i64), Arc<Admitted>>> {
        let mut out = HashMap::new();
        let mut missing = std::collections::BTreeSet::new();
        for (rel, part) in keys {
            // Pin pending values before SQLite can catch up and release the writer's copy.
            // Some(None) is a queued deletion, not a reason to read an older durable row.
            if let Some(pending) = self.pending::<Option<Arc<Admitted>>>(&frame_key(space, rel, *part)) {
                if let Some(frame) = pending.as_ref() { out.insert((rel.clone(), *part), Arc::clone(frame)); }
            } else {
                missing.insert((rel.clone(), *part));
            }
        }
        if missing.is_empty() { return Ok(out); }
        // Read before the first wave (`preload_frames_live`), or written by this process since: from memory. Inside
        // the waves no SQL is run (decision 0104), so a key the snapshot does not hold is absent.
        if snapshot && let Some(snapshot) = self.snapshot() {
            let snapshot = snapshot.lock().expect("the frames' snapshot");
            if let Some(held) = snapshot.get(space) {
                for key in missing {
                    if let Some(frame) = held.get(&key) {
                        out.insert(key, Arc::clone(frame));
                    } else if let Some((at, left)) = &snapshot.left_out
                        && at == space
                        && left.contains(&key.0)
                    {
                        eprintln!("RERAIL_FRAMES_TRACE a frame left out by the preload is read: {} part {}", key.0.name, key.1);
                    }
                }
            }
            return Ok(out);
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let missing: Vec<_> = missing.into_iter().collect();
        let mut chained: Vec<(Rel, u32)> = Vec::new();
        let mut maps = super::segments::SegmentMaps::new(frame_dir(&self.database_path));
        for batch in missing.chunks(READ_BATCH) {
            let values = (0..batch.len()).map(|i| format!("(?{}, ?{}, ?{})", 2 + 3*i, 3 + 3*i, 4 + 3*i)).collect::<Vec<_>>().join(",");
            let columns = if payload { ", f.rows, f.types, f.data, f.seg, f.seg_at, f.seg_len" } else { "" };
            // Keep demand rows outside full-key probes; a space-first scan multiplies retained rows by demands.
            let sql = format!("WITH demanded(rel, ident, part) AS (VALUES {values}) \
                SELECT f.rel, f.ident, f.part, f.digest, f.reads{columns} FROM demanded d \
                CROSS JOIN frames f ON f.space = ?1 AND f.rel = d.rel AND f.ident = d.ident AND f.part = d.part");
            let mut args: Vec<rusqlite::types::Value> = vec![space.to_vec().into()];
            for (rel, part) in batch { args.extend([rel.name.clone().into(), rel.ident.to_vec().into(), (*part).into()]); }
            let mut st = connection.prepare_cached(&sql)?;
            let frames = st.query_map(rusqlite::params_from_iter(args), |row| {
                let ident: Vec<u8> = row.get(1)?;
                let ident: [u8; 32] = ident.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let (reads, k) = decode_reads_chain(&row.get::<_, Vec<u8>>(4)?);
                let data = if payload {
                    let Some(bytes) = row_bytes(&mut maps, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?) else { return Ok(None) };
                    Data::Encoded { n: row.get::<_, i64>(5)? as usize, types: row.get(6)?, bytes, store: Some(self.atoms_handle()) }
                } else {
                    Data::Metadata
                };
                Ok(Some(((Rel { name: row.get(0)?, ident }, row.get(2)?), k, Arc::new(Admitted { stored: Stored { digest: row.get::<_, i64>(3)? as u64, reads }, data }))))
            })?;
            for frame in frames {
                let Some((key, k, frame)) = frame? else { continue };
                if k > 0 && key.1 == 0 {
                    chained.push((key.0.clone(), k));
                }
                out.insert(key, frame);
            }
        }
        // A base frame with deltas after it (decision 0117 step 4) is read with them: the deltas of a batch of such
        // frames in one statement.
        if payload && !chained.is_empty() {
            let mut found: HashMap<Rel, Vec<(i64, Arc<Admitted>)>> = HashMap::new();
            for batch in chained.chunks(READ_BATCH) {
                let values = (0..batch.len()).map(|i| format!("(?{}, ?{}, ?{})", 2 + 3 * i, 3 + 3 * i, 4 + 3 * i)).collect::<Vec<_>>().join(",");
                let sql = format!("WITH demanded(rel, ident, k) AS (VALUES {values}) \
                    SELECT f.rel, f.ident, f.part, f.digest, f.reads, f.rows, f.types, f.data, f.seg, f.seg_at, f.seg_len FROM demanded d \
                    CROSS JOIN frames f ON f.space = ?1 AND f.rel = d.rel AND f.ident = d.ident AND f.part >= -d.k AND f.part < 0");
                let mut args: Vec<rusqlite::types::Value> = vec![space.to_vec().into()];
                for (rel, k) in batch { args.extend([rel.name.clone().into(), rel.ident.to_vec().into(), (*k as i64).into()]); }
                let mut st = connection.prepare_cached(&sql)?;
                let rows = st.query_map(rusqlite::params_from_iter(args), |row| {
                    let ident: Vec<u8> = row.get(1)?;
                    let ident: [u8; 32] = ident.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                    let Some(bytes) = row_bytes(&mut maps, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?) else { return Ok(None) };
                    Ok(Some((Rel { name: row.get(0)?, ident }, row.get::<_, i64>(2)?, Arc::new(Admitted {
                        stored: Stored { digest: row.get::<_, i64>(3)? as u64, reads: decode_reads(&row.get::<_, Vec<u8>>(4)?) },
                        data: Data::Encoded { n: row.get::<_, i64>(5)? as usize, types: row.get(6)?, bytes, store: Some(self.atoms_handle()) },
                    }))))
                })?;
                for row in rows {
                    let Some((rel, part, frame)) = row? else { continue };
                    if is_delta(part, &frame) {
                        found.entry(rel).or_default().push((part, frame));
                    }
                }
            }
            for (rel, mut deltas) in found {
                deltas.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
                let deltas: Vec<Arc<Admitted>> = deltas.into_iter().map(|(_, f)| f).collect();
                let base = Arc::clone(&out[&(rel.clone(), 0)]);
                let mut whole = base.clone_frame();
                for d in deltas {
                    let rows = d.rows()?;
                    let gone = rows.rows.chunks_exact(rows.types.len().max(1)).take(rows.n).filter(|r| r[0] < 0).count();
                    whole = whole.with_delta(base.stored.clone(), d, gone, rows.n - gone);
                }
                out.insert((rel, 0), Arc::new(whole));
            }
        }
        drop(connection);
        hold_symbols(&self.atoms, out.values())?;
        Ok(out)
    }

    /// The deltas after `rel`'s frame in `space` (decision 0117 step 4): how many, and their rows.
    pub fn frame_chain(&self, space: &[u8; 32], rel: &Rel) -> (u32, usize) {
        let key = (rel.clone(), 0);
        self.read_frame_parts(space, std::slice::from_ref(&key), true).ok().and_then(|m| m.get(&key).map(|f| f.chain())).unwrap_or((0, 0))
    }

    /// The rows of `rel`'s frame `part` in `space`.
    pub fn frame_rows(&self, space: &[u8; 32], rel: &Rel, part: i64) -> StoreResult<Option<Rows>> {
        self.admit_frames(space, std::slice::from_ref(rel), part)?.get(rel).map(|f| f.rows()).transpose()
    }

    /// The frames of `rel` in `space` at each of `parts`, admitted (`admit_frame_parts`). A part with no frame is absent.
    pub fn admit_parts_of(&self, space: &[u8; 32], rel: &Rel, parts: &[i64]) -> StoreResult<HashMap<i64, Arc<Admitted>>> {
        let keys: Vec<(Rel, i64)> = parts.iter().map(|p| (rel.clone(), *p)).collect();
        Ok(self.admit_frame_parts(space, &keys)?.into_iter().map(|((_, p), f)| (p, f)).collect())
    }

    /// `admit_parts_of`, from the store's file past the snapshot (queued writes first): a memo's entries looked up
    /// outside the waves, which the snapshot does not hold (`preload_frames_live`).
    pub fn admit_parts_stored(&self, space: &[u8; 32], rel: &Rel, parts: &[i64]) -> StoreResult<HashMap<i64, Arc<Admitted>>> {
        let keys: Vec<(Rel, i64)> = parts.iter().map(|p| (rel.clone(), *p)).collect();
        Ok(self.read_frame_parts_from(space, &keys, true, false)?.into_iter().map(|((_, p), f)| (p, f)).collect())
    }

    /// Every stored frame of `rel` in `space`, by part, admitted (queued writes are not among them).
    pub fn admit_all_parts(&self, space: &[u8; 32], rel: &Rel) -> StoreResult<Vec<(i64, Arc<Admitted>)>> {
        if let Some(snapshot) = self.snapshot() {
            let snapshot = snapshot.lock().expect("the frames' snapshot");
            let mut parts: Vec<(i64, Arc<Admitted>)> = snapshot.get(space).into_iter().flatten().filter(|((r, _), _)| r == rel).map(|((_, p), f)| (*p, Arc::clone(f))).collect();
            parts.sort_by_key(|(p, _)| *p);
            return Ok(parts);
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached(
            "SELECT part, digest, reads, rows, types, data, seg, seg_at, seg_len FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 ORDER BY part",
        )?;
        let atoms = self.atoms_handle();
        let mut maps = super::segments::SegmentMaps::new(frame_dir(&self.database_path));
        let rows = st.query_map(params![&space[..], rel.name, &rel.ident[..]], |row| admitted_part(row, &atoms, &mut maps))?;
        let parts: Vec<(i64, Arc<Admitted>)> = rows.filter_map(Result::transpose).collect::<Result<_, _>>()?;
        drop(st);
        drop(connection);
        hold_symbols(&self.atoms, parts.iter().map(|(_, f)| f))?;
        Ok(parts)
    }

    /// Forgets every frame of `space` and its count of runs: the space of a program no run will run again.
    pub fn drop_frame_space(&self, space: &[u8; 32]) -> StoreResult<usize> {
        if let Some(snapshot) = self.snapshot() {
            snapshot.lock().expect("the frames' snapshot").remove(space);
        }
        self.immediate_transaction_once(|t| {
            let n = t.execute_cached("DELETE FROM frames WHERE space = ?1", [&space[..]])?;
            t.execute_cached("DELETE FROM frame_part_used WHERE space = ?1", [&space[..]])?;
            t.execute_cached("DELETE FROM frame_spaces WHERE space = ?1", [&space[..]])?;
            Ok(n)
        })
    }

    /// `keep_frames`, queued behind the run (decision 0102): encoded here, as each is queued, and written by the store's
    /// writer. The snapshot and the queued write share the encoded frame. Queued as rows, a frame's rows were held by
    /// the queue while the writer encoded a copy of them: on internal_dead_code the run's last writes held 309 MB of
    /// rows beside 138 MB of their encoding (and the kept part's own rows they were copied from).
    pub fn queue_frames(&self, space: [u8; 32], puts: Vec<Put>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_encoded_retagged(space, puts.into_iter().map(Put::encoded).collect(), Vec::new(), Vec::new(), gone, used)
    }

    /// `queue_frames` of frames encoded as they were made (`Put::encoded`): a caller that makes many holds each one's
    /// rows only until it is encoded.
    /// Records, behind the run, that `program` holds `rels` in `space` (`frame_program_used`, decision 0302), when
    /// it does not hold there yet (the snapshot's, read before the first wave, and this process's writes since): a
    /// program that reuses its predecessor's frames puts nothing, and the predecessor superseded, nothing else would
    /// keep them. Whether it queued the holding. No snapshot (no run read the store): queued, and the writer writes
    /// only what changed.
    pub fn hold_frames(&self, space: [u8; 32], rels: Vec<Rel>, run: u64, program: [u8; 32]) -> bool {
        if let Some(s) = self.snapshot()
            && !s.lock().expect("the frames' snapshot").holding.insert((space, program))
        {
            return false;
        }
        self.queue_encoded(space, Vec::new(), Vec::new(), Some((rels, run, program)));
        true
    }

    pub fn queue_encoded(&self, space: [u8; 32], frames: Vec<Encoded>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_encoded_retagged(space, frames, Vec::new(), Vec::new(), gone, used)
    }

    /// `queue_frames`, with `retags` (frames whose rows stay, with their reads told again: no rows are written) and
    /// `deltas` (a relation's rows gone and added, written after its frame: decision 0117 step 4). A frame put whole
    /// (part 0) ends the deltas after it.
    pub fn queue_frames_retagged(&self, space: [u8; 32], puts: Vec<Put>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_encoded_retagged(space, puts.into_iter().map(Put::encoded).collect(), retags, deltas, gone, used)
    }

    /// `queue_frames_retagged` of frames encoded as they were made (`Put::encoded`); its deltas are encoded here.
    pub fn queue_encoded_retagged(&self, space: [u8; 32], frames: Vec<Encoded>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_encoded_as(space, frames, retags, deltas, gone, used, super::write_back::Syms::Mapped, None)
    }

    /// A placer for a write of about `entries` frames, about `estimate` bytes (`queue_placed_retagged`).
    pub fn frame_placer(&self, entries: usize, estimate: usize) -> FramePlacer {
        FramePlacer { w: std::sync::Mutex::new(super::segments::SegmentWriter::new(&frame_dir(&self.database_path), entries.max(1), rerail_blocks::journal::reserve_for(entries.max(1), std::iter::once(estimate), super::segments::ALIGN))) }
    }

    /// `queue_encoded_retagged` of frames some of which `placer` holds placed (`Put::placed_as`): the others are
    /// placed in its segments after them.
    pub fn queue_placed_retagged(&self, space: [u8; 32], frames: Vec<Encoded>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>, placer: FramePlacer) {
        self.queue_encoded_as(space, frames, retags, deltas, gone, used, super::write_back::Syms::Mapped, Some(placer))
    }

    /// `queue_encoded_retagged` for frames keyed by this process's numbering of symbols: rows whose untyped columns,
    /// parts or digests are of atoms (the retained part's documents and granules). The store can map a typed value,
    /// not a key: after a fork (decision 0170) they are not written, and the next run derives them again.
    pub fn queue_encoded_keyed_by_symbols(&self, space: [u8; 32], frames: Vec<Encoded>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_encoded_as(space, frames, retags, deltas, gone, used, super::write_back::Syms::Unmapped, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_encoded_as(&self, space: [u8; 32], frames: Vec<Encoded>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>, syms: super::write_back::Syms, placer: Option<FramePlacer>) {
        // Each delta after its relation's frame as it is now (in memory, queued or stored).
        let mut delta_frames: Vec<Encoded> = Vec::new();
        let mut heads: Vec<(Rel, u64, Vec<(String, u64)>)> = Vec::new();
        // Each delta's relation as it is now, its new head and its rows' counts: composed with the delta once it is
        // placed (below).
        let mut chained: Vec<(Rel, Arc<Admitted>, Stored, usize, usize)> = Vec::new();
        for d in deltas {
            let w = d.types.len();
            let key = (d.rel.clone(), 0);
            let Some(now) = self.read_frame_parts(&space, std::slice::from_ref(&key), true).ok().and_then(|mut m| m.remove(&key)) else { continue };
            if w == 0 {
                continue;
            }
            let (k, _) = now.chain();
            let (ng, na) = (d.gone.len() / w, d.added.len() / w);
            let mut rows = Vec::with_capacity((ng + na) * (w + 1));
            for r in d.gone.chunks_exact(w) {
                rows.push(-1);
                rows.extend_from_slice(r);
            }
            for r in d.added.chunks_exact(w) {
                rows.push(1);
                rows.extend_from_slice(r);
            }
            let mut types = vec![DELTA];
            types.extend_from_slice(&d.types);
            let delta = Put { rel: d.rel.clone(), part: delta_part(k + 1), digest: 0, reads: Vec::new(), rows: Rows { types, rows, n: ng + na } }.encoded();
            chained.push((d.rel.clone(), now, Stored { digest: d.digest, reads: d.reads.clone() }, ng, na));
            let mut stored_reads = d.reads.clone();
            stored_reads.push((CHAIN.to_owned(), (k + 1) as u64));
            heads.push((d.rel, d.digest, stored_reads));
            delta_frames.push(delta);
        }
        let bytes: usize = frames.iter().chain(&delta_frames).map(|e| e.frame.encoded_bytes()).sum();
        // The frames' bytes in segments of their own (decision 0295 §1): placed here, by the run, so the snapshot,
        // the queue's readers and the writer share them mapped, the OS's pages, not the run's memory.
        let (frames, delta_frames, segments) = self.place_frames(frames, delta_frames, bytes, placer);
        let composed: Vec<(Rel, Arc<Admitted>)> =
            chained.into_iter().zip(&delta_frames).map(|((rel, now, head, ng, na), delta)| (rel, Arc::new(now.with_delta(head, Arc::clone(&delta.frame), ng, na)))).collect();
        // What a large write of frames holds, by relation (profiling only): what makes the store's writes large.
        if bytes > 4 << 20 && (std::env::var_os("RERAIL_PROFILE").is_some() || std::env::var_os("RERAIL_WRITES").is_some()) {
            let mut by: HashMap<&str, (usize, usize, usize)> = HashMap::new();
            for f in &frames {
                let e = by.entry(f.rel.name.as_str()).or_default();
                e.0 += 1;
                e.1 += f.frame.len().unwrap_or(0);
                e.2 += f.frame.encoded_bytes();
            }
            let mut by: Vec<_> = by.into_iter().collect();
            by.sort_by(|a, b| b.1.2.cmp(&a.1.2));
            let top: Vec<String> = by.iter().take(15).map(|(r, (f, n, b))| format!("{r}: {f} frames, {n} rows, {:.1} MB", *b as f64 / 1048576.0)).collect();
            eprintln!("rerail: frames queued: {:.1} MB in {} frames of {} relations; by relation: {}", bytes as f64 / 1048576.0, frames.len(), by.len(), top.join("; "));
            if let Some(dir) = std::env::var_os("RERAIL_PROFILE") {
                let mut tsv = String::from("relation\tframes\trows\tbytes\n");
                for (r, (f, n, b)) in &by {
                    tsv.push_str(&format!("{r}\t{f}\t{n}\t{b}\n"));
                }
                let _ = std::fs::write(std::path::Path::new(&dir).join(format!("frames-{}.tsv", bytes)), tsv);
            }
        }
        let mut gone = gone;
        gone.extend(self.ended_deltas(&space, frames.iter().map(|f| (&f.rel, f.part))));
        if let (Some((_, _, program)), Some(s)) = (&used, self.snapshot()) {
            s.lock().expect("the frames' snapshot").holding.insert((space, *program));
        }
        // Nothing to write is no write: a warm run's kept part queues no empty job (decision 0197).
        if frames.is_empty() && delta_frames.is_empty() && retags.is_empty() && heads.is_empty() && gone.is_empty() && used.is_none() {
            return;
        }
        let mut keys: Vec<(super::write_back::Key, Option<Arc<dyn std::any::Any + Send + Sync>>)> = frames
            .iter()
            .map(|f| (frame_key(&space, &f.rel, f.part), Some(Arc::new(Some(Arc::clone(&f.frame))) as Arc<dyn std::any::Any + Send + Sync>)))
            .collect();
        keys.extend(gone.iter().map(|(rel, part)| (frame_key(&space, rel, *part), Some(Arc::new(None::<Arc<Admitted>>) as Arc<dyn std::any::Any + Send + Sync>))));
        keys.extend(composed.iter().map(|(rel, f)| (frame_key(&space, rel, 0), Some(Arc::new(Some(Arc::clone(f))) as Arc<dyn std::any::Any + Send + Sync>))));
        // Memory is ahead of the store: the snapshot holds what is queued (`preload_frames`), the queued frames
        // themselves.
        if let Some(snapshot) = self.snapshot() {
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            let held = snapshot.entry(space).or_default();
            for (rel, f) in &composed {
                held.insert((rel.clone(), 0), Arc::clone(f));
            }
            // A retagged frame keeps its rows (shared) and digest, with its new reads.
            for r in &retags {
                if let Some(was) = held.get(&(r.rel.clone(), r.part)).cloned() {
                    let frame = Arc::new(Admitted { stored: Stored { digest: was.stored.digest, reads: r.reads.clone() }, data: was.data.clone() });
                    keys.push((frame_key(&space, &r.rel, r.part), Some(Arc::new(Some(Arc::clone(&frame))) as Arc<dyn std::any::Any + Send + Sync>)));
                    held.insert((r.rel.clone(), r.part), frame);
                }
            }
            for f in &frames {
                held.insert((f.rel.clone(), f.part), Arc::clone(&f.frame));
            }
            for key in &gone {
                held.remove(key);
            }
            drop(snapshot);
            // The confirmations as the writer will leave them (`write_frames`).
            if let Some(snapshot) = self.snapshot() {
                let mut snapshot = snapshot.lock().expect("the frames' snapshot");
                let map = snapshot.confirmed.entry(space).or_default();
                for f in frames.iter().chain(&delta_frames) {
                    map.remove(&confirm_key(&f.rel, f.part));
                }
                for (rel, _, _) in &heads {
                    map.remove(&confirm_key(rel, 0));
                }
                for (rel, part) in &gone {
                    map.remove(&confirm_key(rel, *part));
                }
                for r in &retags {
                    map.insert(confirm_key(&r.rel, r.part), reads_hash(&r.reads));
                }
            }
        }
        // The queue's measure: what it holds in memory, the frames not placed (the placed ones are the OS's pages).
        let held: usize = frames.iter().chain(&delta_frames).filter(|e| e.place.is_none()).map(|e| e.frame.encoded_bytes() + 64).sum::<usize>() + frames.len() * 64;
        let write = FramesWrite { space, frames, delta_frames, retags, heads, gone, used, segments };
        self.write_back_syms(syms, "frames", held, keys, super::write_back::Work(super::write_back::Kind::Frames(Box::new(write))));
    }

    /// The frame segments' bytes, and the bytes their rows name: what compaction would leave.
    pub fn frame_segment_bytes(&self) -> StoreResult<(u64, u64)> {
        let (segs, named): (Vec<i64>, i64) = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let segs = connection.prepare_cached("SELECT seg FROM frame_segments")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            (segs, connection.query_row_cached("SELECT coalesce(sum(seg_len), 0) FROM frames WHERE seg != 0", [], |r| r.get(0))?)
        };
        let dir = frame_dir(&self.database_path);
        let held = segs.iter().map(|&s| std::fs::metadata(super::segments::segment_path(&dir, s)).map_or(0, |m| m.len())).sum();
        Ok((held, named as u64))
    }

    /// Copies every placed frame's bytes to one new segment and removes the others, where the bytes no row names (frames
    /// written again, forgotten, collected) are more than half the segments and at least `floor`: one write
    /// transaction. A frame whose segment is not there is forgotten (derived again). Returns the bytes freed. A reader
    /// that mapped an old segment reads it on; one that finds a row from before misses.
    pub fn compact_frame_segments(&self, floor: u64) -> StoreResult<u64> {
        let (held, named) = self.frame_segment_bytes()?;
        if held < floor || held.saturating_sub(named) < held / 2 {
            return Ok(0);
        }
        let dir = frame_dir(&self.database_path);
        let old = self.immediate_transaction_once(|t| {
            let segs: Vec<i64> = t.prepare_cached("SELECT seg FROM frame_segments")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            type Placed = (Vec<u8>, String, Vec<u8>, i64, super::segments::Place);
            let rows: Vec<Placed> = t
                .prepare_cached("SELECT space, rel, ident, part, seg, seg_at, seg_len FROM frames WHERE seg != 0 ORDER BY seg, seg_at")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, super::segments::Place { seg: r.get(4)?, at: r.get::<_, i64>(5)? as u64, len: r.get::<_, i64>(6)? as usize })))?
                .collect::<Result<_, _>>()?;
            let mut maps = super::segments::SegmentMaps::new(dir.clone());
            let mut kept: Vec<(&Placed, super::segments::SegmentBytes)> = Vec::new();
            for row in &rows {
                match maps.at(row.4) {
                    Some(bytes) => kept.push((row, bytes)),
                    None => {
                        t.execute_cached("DELETE FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4", params![row.0, row.1, row.2, row.3])?;
                        t.execute_cached("DELETE FROM frame_part_used WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4", params![row.0, row.1, row.2, row.3])?;
                    }
                }
            }
            t.execute_cached("DELETE FROM frame_segments", [])?;
            if kept.is_empty() {
                return Ok(segs);
            }
            let entries: Vec<(&[u8], i64, &[u8; 32])> = kept.iter().map(|(row, b)| (b.bytes(), row.3, &[0u8; 32])).collect();
            let mut placed = super::segments::write_segments(&dir, &entries)?;
            if placed.files().len() != 1 {
                return Err(super::StoreError::Invalid("the frames' compaction made more than one segment".into()));
            }
            let seg = segs.iter().copied().max().unwrap_or(0) + 1;
            t.execute_cached("INSERT INTO frame_segments(seg) VALUES (?1)", [seg])?;
            for (i, (row, _)) in kept.iter().enumerate() {
                t.execute_cached(
                    "UPDATE frames SET seg = ?5, seg_at = ?6 WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4",
                    params![row.0, row.1, row.2, row.3, seg, placed.placed(i).1 as i64],
                )?;
            }
            placed.name(&dir, &[seg])?;
            Ok(segs)
        })?;
        for seg in old {
            let _ = std::fs::remove_file(super::segments::segment_path(&dir, seg));
        }
        let (now, _) = self.frame_segment_bytes()?;
        Ok(held.saturating_sub(now))
    }

    /// `frames` and `deltas` with their bytes placed in new segments of the store's frame directory, each frame then
    /// reading them there (`Bytes::Mapped`), and the segments, sealed, for the writer. Frames without bytes, or a
    /// write whose segments cannot be made (no room on disk: the frames stay in memory, written inline), are left.
    /// With `placer`, the frames it placed already (`Put::placed_as`) are its entries, and the others are placed after
    /// them in its segments; one no segment takes stays on the heap, written from there.
    fn place_frames(&self, frames: Vec<Encoded>, deltas: Vec<Encoded>, bytes: usize, placer: Option<FramePlacer>) -> (Vec<Encoded>, Vec<Encoded>, Option<super::segments::SealedSegments>) {
        let encoded = |e: &Encoded| e.place.is_none() && matches!(e.frame.data, Data::Encoded { store: None, .. });
        let n = frames.iter().chain(&deltas).filter(|e| encoded(e)).count();
        let mut w = match placer {
            Some(p) => p.w.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner),
            None if n == 0 => return (frames, deltas, None),
            None => super::segments::SegmentWriter::new(&frame_dir(&self.database_path), n, rerail_blocks::journal::reserve_for(n, std::iter::once(bytes), super::segments::ALIGN)),
        };
        if n == 0 && w.len() == 0 {
            return (frames, deltas, None);
        }
        let mut at = w.len();
        let mut room = true;
        let mut placed = Vec::with_capacity(frames.len() + deltas.len());
        for e in frames.iter().chain(&deltas) {
            if e.place.is_some() {
                placed.push(e.place);
            } else if room && let Data::Encoded { bytes, store: None, .. } = &e.frame.data {
                if w.put(bytes.len(), e.part, &[0; 32], |out| out.copy_from_slice(bytes)).is_err() {
                    room = false;
                    placed.push(None);
                    continue;
                }
                placed.push(Some(at));
                at += 1;
            } else {
                placed.push(None);
            }
        }
        let segments = w.seal();
        let mut placed = placed.into_iter();
        let mut place = |e: Encoded| -> Encoded {
            let Some(i) = placed.next().flatten() else { return e };
            let Data::Encoded { types, n, .. } = &e.frame.data else { return e };
            let frame = Admitted { stored: e.frame.stored.clone(), data: Data::Encoded { types: types.clone(), n: *n, bytes: Bytes::Mapped(segments.entry(i)), store: None } };
            Encoded { rel: e.rel, part: e.part, frame: Arc::new(frame), place: Some(i) }
        };
        let frames: Vec<Encoded> = frames.into_iter().map(&mut place).collect();
        let deltas: Vec<Encoded> = deltas.into_iter().map(&mut place).collect();
        (frames, deltas, Some(segments))
    }

    /// Writes `puts` into `space` (each replacing its frame) and forgets `gone`. With `used`, the program's relations
    /// in the space are recorded with it (`frame_program_used`), for collection (`collect_unreachable_frames`).
    pub fn keep_frames(&self, space: &[u8; 32], puts: &[Put], gone: &[(Rel, i64)], used: Option<(&[Rel], u64, [u8; 32])>) -> StoreResult<()> {
        let mut gone = gone.to_vec();
        gone.extend(self.ended_deltas(space, puts.iter().map(|p| (&p.rel, p.part))));
        // Written now, not queued: the confirmations this process holds lose these frames now too.
        if let Some(snapshot) = self.snapshot()
            && let Some(map) = snapshot.lock().expect("the frames' snapshot").confirmed.get_mut(space)
        {
            for p in puts {
                map.remove(&confirm_key(&p.rel, p.part));
            }
            for (rel, part) in &gone {
                map.remove(&confirm_key(rel, *part));
            }
        }
        let blocks: Vec<Vec<u8>> = puts.iter().map(|p| super::cross_rows::encode(p.rows.types.len(), &p.rows.rows)).collect();
        let stored: Vec<Stored> = puts.iter().map(|p| Stored { digest: p.digest, reads: p.reads.clone() }).collect();
        let rows: Vec<Framed> = puts
            .iter()
            .zip(&blocks)
            .zip(&stored)
            .map(|((p, bytes), stored)| Framed { rel: &p.rel, part: p.part, stored, types: &p.rows.types, n: p.rows.n, bytes, place: None })
            .collect();
        let written = self.write_frames(space, &rows, 0, &[], &[], &gone, used);
        drop(rows);
        // The snapshot holds what is written, the blocks moved into it once written. Cloned into it before the write,
        // the blocks were held twice while the transaction ran.
        if let Some(snapshot) = self.snapshot() {
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            let held = snapshot.entry(*space).or_default();
            for ((p, block), stored) in puts.iter().zip(blocks).zip(stored) {
                held.insert((p.rel.clone(), p.part), Arc::new(Admitted { stored, data: Data::Encoded { types: p.rows.types.clone(), n: p.rows.n, bytes: Bytes::Heap(Arc::new(block)), store: None } }));
            }
            for key in &gone {
                held.remove(key);
            }
        }
        written
    }

    /// The delta frames that `puts` end (decision 0117 step 4): those after a frame put whole, as the snapshot holds
    /// them. Without a snapshot none is named; its reads name no deltas, so they are not read, and the next delta
    /// writes over them.
    fn ended_deltas<'p>(&self, space: &[u8; 32], puts: impl Iterator<Item = (&'p Rel, i64)>) -> Vec<(Rel, i64)> {
        let Some(snapshot) = self.snapshot() else { return Vec::new() };
        let snapshot = snapshot.lock().expect("the frames' snapshot");
        let Some(held) = snapshot.get(space) else { return Vec::new() };
        let mut out = Vec::new();
        for (rel, _) in puts.filter(|(_, part)| *part == 0) {
            if let Some(f) = held.get(&(rel.clone(), 0)) {
                out.extend((1..=f.chain().0).map(|i| (rel.clone(), delta_part(i))));
            }
        }
        out
    }

    /// Writes encoded `rows` into `space` (each replacing its frame; the last `deltas` of them delta frames), with
    /// `retags`, the deltas' `heads`, and `gone` forgotten, in one transaction; with `used`, as `keep_frames`.
    #[allow(clippy::too_many_arguments)]
    fn write_frames(&self, space: &[u8; 32], rows: &[Framed], deltas: usize, retags: &[Retag], heads: &[(Rel, u64, Vec<(String, u64)>)], gone: &[(Rel, i64)], used: Option<(&[Rel], u64, [u8; 32])>) -> StoreResult<()> {
        self.write_frames_placed(space, rows, deltas, retags, heads, gone, used, None)
    }

    /// `write_frames`, of frames placed in `segments` (`place_frames`): those written back first, numbered in the
    /// transaction, and each placed frame's row a pointer into them (decision 0295 §4); renamed to their numbers once
    /// the rows commit. A forked process writes every frame's bytes inline: the writer renumbers their symbols.
    #[allow(clippy::too_many_arguments)]
    fn write_frames_placed(&self, space: &[u8; 32], rows: &[Framed], deltas: usize, retags: &[Retag], heads: &[(Rel, u64, Vec<(String, u64)>)], gone: &[(Rel, i64)], used: Option<(&[Rel], u64, [u8; 32])>, mut segments: Option<&mut super::segments::SealedSegments>) -> StoreResult<()> {
        let timed = std::env::var_os("RERAIL_WRITES").is_some();
        let files = match segments.as_deref_mut() {
            Some(s) => {
                s.write_back()?;
                s.files()
            }
            None => Vec::new(),
        };
        let numbers = self.immediate_transaction_once(|t| {
            let mut at = std::time::Instant::now();
            let mut lap = |what: &str| if timed { eprintln!("RERAIL_WRITES frames {what}: {:.3} s", at.elapsed().as_secs_f64()); at = std::time::Instant::now(); };
            // The space's counter stamps what is put (`frame_part_used`): a run's first program counts it, and a write
            // of no run's (the memo's entries) counts itself, so a path's contents are ordered by when they were put
            // Only a write that puts counts: a use is not marked (decision 0227).
            let run = match used {
                Some((_, token, _)) => count_run(t, space, token)?,
                None if !rows.is_empty() => count_run(t, space, 0)?,
                None => {
                    t.execute_cached("INSERT INTO frame_spaces(space, runs) VALUES (?1, 0) ON CONFLICT(space) DO NOTHING", [&space[..]])?;
                    t.query_row_cached("SELECT runs FROM frame_spaces WHERE space = ?1", [&space[..]], |r| r.get(0))?
                }
            };
            // Forked (decision 0170): the frames were encoded as they were made, in this process's ids; their symbol
            // columns are written as the store numbers them, mapped in this transaction. Not forked: as encoded.
            let mapped: Vec<Option<Vec<u8>>> = if self.atoms.forked() {
                rows.iter().map(|f| f.store_block(|v| self.atoms.to_store(t, super::ProcessIds::of_process(v)).map(drop))).collect::<StoreResult<_>>()?
            } else {
                Vec::new()
            };
            // The segments, numbered: each placed frame a pointer into them, its row holding no bytes.
            let mut numbers: Vec<i64> = Vec::new();
            if !self.atoms.forked() {
                for _ in &files {
                    let seg: i64 = t.query_row_cached("SELECT coalesce(max(seg), 0) + 1 FROM frame_segments", [], |r| r.get(0))?;
                    t.execute_cached("INSERT INTO frame_segments(seg) VALUES (?1)", [seg])?;
                    numbers.push(seg);
                }
            }
            let pointer = |f: &Framed| -> Option<(i64, i64, i64)> {
                let i = f.place?;
                let (s, at, len) = segments.as_deref().map(|s| s.placed(i))?;
                Some((*numbers.get(s)?, at as i64, len as i64))
            };
            let data_of = |i: usize, f: &Framed| -> (Vec<u8>, (i64, i64, i64)) {
                match pointer(f) {
                    Some(p) => (Vec::new(), p),
                    None => (mapped.get(i).and_then(Option::as_deref).unwrap_or(f.bytes).to_vec(), (0, 0, 0)),
                }
            };
            // A frame put whole has no deltas after it: its reads name none (`CHAIN`), so those left are not read.
            // Twelve bound values per frame, plus shared space and run, stay below 999 parameters.
            // Each statement publishes a bounded batch. No frame launches its own SQL execution.
            for (c, batch) in rows.chunks(80).enumerate() {
                let values = (0..batch.len()).map(|i| {
                    let p=3+12*i;
                    format!("(?1,?{p},?{},?{},?{},?{},?{},?{},?{},?{},?2,?{},?{},?{})",p+1,p+2,p+3,p+4,p+5,p+6,p+7,p+8,p+9,p+10,p+11)
                }).collect::<Vec<_>>().join(",");
                let sql=format!("INSERT INTO frames(space,rel,ident,part,digest,reads,width,rows,types,data,used,seg,seg_at,seg_len) VALUES {values} \
                    ON CONFLICT(space,rel,ident,part) DO UPDATE SET digest=excluded.digest, reads=excluded.reads, \
                    width=excluded.width, rows=excluded.rows, types=excluded.types, data=excluded.data, used=excluded.used, \
                    seg=excluded.seg, seg_at=excluded.seg_at, seg_len=excluded.seg_len");
                let mut args: Vec<rusqlite::types::Value>=vec![space.to_vec().into(),run.into()];
                for (k, f) in batch.iter().enumerate() {
                    let (data, (seg, at, len)) = data_of(c * 80 + k, f);
                    args.extend([f.rel.name.clone().into(),f.rel.ident.to_vec().into(),f.part.into(),(f.stored.digest as i64).into(),
                        encode_reads(&f.stored.reads).into(),(f.types.len() as i64).into(),(f.n as i64).into(),f.types.to_vec().into(),data.into(),
                        seg.into(),at.into(),len.into()]);
                }
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
            }
            // Each frame's use, apart from its record (`frame_part_used`): three values a frame, so 300 a statement.
            for batch in rows.chunks(300) {
                let values = (0..batch.len()).map(|i| format!("(?1,?{},?{},?{},?2)", 3 + 3 * i, 4 + 3 * i, 5 + 3 * i)).collect::<Vec<_>>().join(",");
                let sql = format!("INSERT INTO frame_part_used(space,rel,ident,part,used) VALUES {values} \
                    ON CONFLICT(space,rel,ident,part) DO UPDATE SET used=excluded.used");
                let mut args: Vec<rusqlite::types::Value> = vec![space.to_vec().into(), run.into()];
                for f in batch {
                    args.extend([f.rel.name.clone().into(), f.rel.ident.to_vec().into(), f.part.into()]);
                }
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
            }
            lap(&format!("{} puts, {} deltas", rows.len() - deltas, deltas));
            // A relation with a delta after its frame: the frame's digest and reads are the relation's now.
            for (rel, digest, reads) in heads {
                t.prepare_cached("UPDATE frames SET digest = ?5, reads = ?6 WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?
                    .execute(rusqlite::params![&space[..], rel.name, &rel.ident[..], 0i64, *digest as i64, encode_reads(reads)])?;
                t.prepare_cached("UPDATE frame_part_used SET used = ?5 WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?
                    .execute(rusqlite::params![&space[..], rel.name, &rel.ident[..], 0i64, run])?;
            }
            // A retagged frame is not written (decision 0117 step 5): its rows, digest and stored reads stay, and the
            // space's confirmations name it with its reads' hash now, one row for the space. A frame put, given a
            // delta's head or forgotten here loses its confirmation in the same transaction: a confirmation never
            // vouches for rows written after it.
            if !(retags.is_empty() && rows.is_empty() && heads.is_empty() && gone.is_empty()) {
                let was: Option<Vec<u8>> = t.query_row_cached("SELECT data FROM frame_confirmed WHERE space = ?1", [&space[..]], |r| r.get(0)).optional()?;
                if was.is_some() || !retags.is_empty() {
                    let mut map = was.as_deref().map(decode_confirmed).unwrap_or_default();
                    let before = map.clone();
                    for f in rows {
                        map.remove(&confirm_key(f.rel, f.part));
                    }
                    for (rel, _, _) in heads {
                        map.remove(&confirm_key(rel, 0));
                    }
                    for (rel, part) in gone {
                        map.remove(&confirm_key(rel, *part));
                    }
                    for r in retags {
                        map.insert(confirm_key(&r.rel, r.part), reads_hash(&r.reads));
                    }
                    if map != before {
                        t.prepare_cached("INSERT OR REPLACE INTO frame_confirmed(space, data) VALUES (?1, ?2)")?.execute(rusqlite::params![&space[..], encode_confirmed(&map)])?;
                    }
                }
            }
            lap(&format!("{} heads, {} retags", heads.len(), retags.len()));
            for batch in gone.chunks(READ_BATCH) {
                let values=(0..batch.len()).map(|i|format!("(?{},?{},?{})",2+3*i,3+3*i,4+3*i)).collect::<Vec<_>>().join(",");
                let sql=format!("WITH demanded(rel,ident,part) AS (VALUES {values}) \
                    DELETE FROM frames WHERE (space,rel,ident,part) IN (SELECT ?1,rel,ident,part FROM demanded)");
                let mut args: Vec<rusqlite::types::Value>=vec![space.to_vec().into()];
                for (rel,part) in batch { args.extend([rel.name.clone().into(),rel.ident.to_vec().into(),(*part).into()]); }
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args.clone()))?;
                let sql=format!("WITH demanded(rel,ident,part) AS (VALUES {values}) \
                    DELETE FROM frame_part_used WHERE (space,rel,ident,part) IN (SELECT ?1,rel,ident,part FROM demanded)");
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
            }
            lap(&format!("{} gone", gone.len()));
            if let Some((used, _, program)) = used {
                // The relations the program derives in the space (decision 0227, reachability): one row a program,
                // written when it first puts there or derives more. No run marks its use: collection keeps a
                // relation's frames while a live program (a plan a kept manifest names) holds it here
                // (`collect_unreachable_frames`). A warm run, which puts nothing, writes nothing.
                let was: Option<Vec<u64>> = t
                    .prepare_cached("SELECT rels FROM frame_program_used WHERE space = ?1 AND program = ?2 AND run = 0")?
                    .query_row(params![&space[..], &program[..]], |r| Ok(decode_keys(&r.get::<_, Vec<u8>>(0)?)))
                    .optional()?;
                let mut keys: Vec<u64> = used.iter().map(used_key).collect();
                keys.extend(was.iter().flatten().copied());
                keys.sort_unstable();
                keys.dedup();
                if was.as_ref() != Some(&keys) {
                    t.prepare_cached("INSERT OR REPLACE INTO frame_program_used(space, program, run, rels) VALUES (?1, ?2, 0, ?3)")?.execute(params![&space[..], &program[..], encode_keys(&keys)])?;
                }
                lap(&format!("{} used", used.len()));
            }
            Ok(numbers)
        })?;
        // The segments under their numbers, once the rows naming them are committed (a forked write named none: its
        // segments go when dropped).
        if let Some(s) = segments
            && !numbers.is_empty()
        {
            s.name(&frame_dir(&self.database_path), &numbers)?;
        }
        Ok(())
    }
}

/// What reachability collected (`collect_unreachable_frames`): memo entries whose content no kept manifest names,
/// and maintained frames of relations no live program derives, with the use rows that went with them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unreachable {
    pub memo: usize,
    pub maintained: usize,
    pub programs: usize,
    /// Of `programs`, those no manifest ever named (not in `corpus_plans` at all): a part whose key its run does not
    /// name, whose frames every decision collects and every run derives again. Should stay 0.
    pub unnamed: usize,
}

impl Unreachable {
    pub fn any(&self) -> bool {
        self.memo + self.maintained + self.programs > 0
    }
}

/// Collects the frames nothing live reaches (decision 0227), in `t` (a write transaction). No run marks what it uses;
/// what is kept follows from the kept manifests (`corpus_runs`'s keep decision):
///
/// - **A memo entry** (a live plan's space, `local_space`) is kept while a kept manifest names its content: its reads
///   hold its document's sha256, and `corpus_live` the kept manifests' sources. A dead plan's whole space goes with
///   the plan (`collect_corpus_pass`).
/// - **A maintained frame** (a part's space) is kept while a live program holds its relation there: a program (a part's
///   key) is live while a kept manifest names it (`corpus_plans`), and the relations it derives in the space are its
///   `frame_program_used` row. A space no live program holds loses every frame.
pub(super) fn collect_unreachable_frames(t: &Connection) -> rusqlite::Result<Unreachable> {
    let mut out = Unreachable::default();
    let live: HashSet<[u8; 32]> = t
        .prepare("SELECT plan_digest FROM corpus_plans WHERE live = 1")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .filter_map(|p| p.ok()?.try_into().ok())
        .collect();
    // The kept manifests' contents, by sha256.
    let contents: HashSet<[u8; 32]> = t
        .prepare(
            "SELECT DISTINCT h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3 FROM corpus_live l \
             JOIN source_objects s ON s.source_id = l.source_id JOIN hashes h ON h.hash_id = s.canonical_hash_id",
        )?
        .query_map([], |r| Ok(super::HashKey { algorithm: r.get(0)?, lanes: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?] }))?
        .filter_map(|k| k.ok()?.sha256_bytes())
        .collect();
    // The memo's entries (decision 0240 §4); and its frames of before it, every one (`MEMO_RELS`).
    let mut gone: Vec<([u8; 32], String, Vec<u8>, i64)> = Vec::new();
    // By the live plans' spaces (a dead plan's space goes whole, `collect_corpus_pass`): the relation leads no index.
    let mut old = 0;
    for plan in &live {
        let space = local_space(plan);
        old += t.execute_cached("DELETE FROM frames WHERE space = ?1 AND rel IN (?2, ?3)", params![&space[..], MEMO_RELS[0], MEMO_RELS[1]])?;
        t.execute_cached("DELETE FROM frame_part_used WHERE space = ?1 AND rel IN (?2, ?3)", params![&space[..], MEMO_RELS[0], MEMO_RELS[1]])?;
    }
    out.memo = old + memo_pack::collect_memo_entries(t, &live, &contents)?;
    // The maintained spaces: each one's live programs' relations.
    let rows: Vec<(Vec<u8>, Vec<u8>, i64, Vec<u8>)> = t
        .prepare("SELECT space, program, run, rels FROM frame_program_used")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut held: HashMap<Vec<u8>, HashSet<u64>> = HashMap::new();
    let mut dead_rows: Vec<(Vec<u8>, Vec<u8>, i64)> = Vec::new();
    for (space, program, run, rels) in rows {
        let entry = held.entry(space.clone()).or_default();
        // A row of a run (before decision 0227) or of a dead program holds nothing.
        if run == 0 && <[u8; 32]>::try_from(program.as_slice()).is_ok_and(|p| live.contains(&p)) {
            entry.extend(decode_keys(&rels));
        } else {
            dead_rows.push((space, program, run));
        }
    }
    out.programs = dead_rows.len();
    let known: HashSet<Vec<u8>> = t.prepare("SELECT plan_digest FROM corpus_plans")?.query_map([], |r| r.get::<_, Vec<u8>>(0))?.collect::<Result<_, _>>()?;
    out.unnamed = dead_rows.iter().filter(|(_, p, run)| *run == 0 && !known.contains(p)).count();
    let before = gone.len();
    for (space, rels) in &held {
        let Ok(space) = <[u8; 32]>::try_from(space.as_slice()) else { continue };
        let mut st = t.prepare_cached("SELECT rel, ident, part FROM frame_part_used WHERE space = ?1")?;
        let rows = st.query_map([&space[..]], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (rel, ident, part) = row?;
            let Ok(id) = <[u8; 32]>::try_from(ident.as_slice()) else { continue };
            if !rels.contains(&used_key(&Rel { name: rel.clone(), ident: id })) {
                gone.push((space, rel, ident, part));
            }
        }
    }
    out.maintained = gone.len() - before;
    // A dev trace (`RERAIL_FRAMES_TRACE`): each dead program row and each space's collected frames.
    if std::env::var_os("RERAIL_FRAMES_TRACE").is_some() {
        let hex = |b: &[u8]| b.iter().take(6).map(|x| format!("{x:02X}")).collect::<String>();
        for (space, program, run) in &dead_rows {
            eprintln!("RERAIL_FRAMES_TRACE dead row: space {} program {} run {run} known {}", hex(space), hex(program), known.contains(program));
        }
        let mut by: HashMap<String, usize> = HashMap::new();
        for (space, rel, _, _) in &gone[before..] {
            *by.entry(format!("{} {rel}", hex(space))).or_default() += 1;
        }
        for (k, n) in by {
            eprintln!("RERAIL_FRAMES_TRACE collected: {k}: {n}");
        }
    }
    for (space, rel, ident, part) in &gone {
        t.prepare_cached("DELETE FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?.execute(params![&space[..], rel, ident, part])?;
        t.prepare_cached("DELETE FROM frame_part_used WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?.execute(params![&space[..], rel, ident, part])?;
    }
    for (space, program, run) in &dead_rows {
        t.prepare_cached("DELETE FROM frame_program_used WHERE space = ?1 AND program = ?2 AND run = ?3")?.execute(params![space, program, run])?;
    }
    Ok(out)
}

/// A frame's key in its space's confirmations: its relation's name and identity, and its part.
pub fn confirm_key(rel: &Rel, part: i64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rel.name.as_bytes().iter().chain(&[0]).chain(&rel.ident).chain(&part.to_le_bytes()) {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The hash a confirmation keeps of a frame's reads (names and digests, in order; no `CHAIN` entry).
pub fn reads_hash(reads: &[(String, u64)]) -> u64 {
    let mut h: u64 = 0x84222325_cbf29ce4;
    for (name, d) in reads {
        for b in name.as_bytes().iter().chain(&[0]) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ d).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 31;
    }
    h
}

fn encode_confirmed(map: &HashMap<u64, u64>) -> Vec<u8> {
    let mut pairs: Vec<(u64, u64)> = map.iter().map(|(k, v)| (*k, *v)).collect();
    pairs.sort_unstable();
    pairs.iter().flat_map(|(k, v)| k.to_le_bytes().into_iter().chain(v.to_le_bytes())).collect()
}

fn decode_confirmed(bytes: &[u8]) -> HashMap<u64, u64> {
    bytes.chunks_exact(16).map(|c| (u64::from_le_bytes(c[..8].try_into().expect("8 bytes")), u64::from_le_bytes(c[8..].try_into().expect("8 bytes")))).collect()
}

/// A frame encoded to be queued (`Put::encoded`, `queue_encoded`): the queue, the snapshot and the writer share it.
pub struct Encoded {
    rel: Rel,
    part: i64,
    frame: Arc<Admitted>,
    /// Where its bytes were placed (`place_frames`): an entry of the call's segments.
    place: Option<usize>,
}

impl Encoded {
    /// The frame as the writer writes it (none for a frame without rows).
    fn framed(&self) -> Option<Framed<'_>> {
        match &self.frame.data {
            Data::Encoded { types, n, bytes, .. } => Some(Framed { rel: &self.rel, part: self.part, stored: &self.frame.stored, types, n: *n, bytes, place: self.place }),
            Data::Metadata | Data::Chain { .. } => None,
        }
    }
}

impl Put {
    /// This frame encoded, its rows dropped.
    pub fn encoded(self) -> Encoded {
        let Put { rel, part, digest, reads, rows } = self;
        let bytes = Self::encode_rows(&rows);
        let Rows { types, n, rows } = rows;
        drop(rows);
        Self::encoded_as(rel, part, digest, reads, types, n, bytes)
    }
    /// Rows' bytes as `encoded` encodes them, for a frame made from them later (`encoded_as`): encoded where the
    /// rows are (a job of their own), and named where the frame's digest and reads are known.
    pub fn encode_rows(rows: &Rows) -> Vec<u8> {
        super::cross_rows::encode(rows.types.len(), &rows.rows)
    }
    /// A frame whose bytes a write's placer holds already (`FramePlacer::place_rows`): queued with that placer
    /// (`queue_placed_retagged`), its bytes are read where they were written.
    pub fn placed_as(rel: Rel, part: i64, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, n: usize, placed: Placed) -> Encoded {
        let data = Data::Encoded { types, n, bytes: Bytes::Heap(Arc::new(Vec::new())), store: None };
        Encoded { rel, part, frame: Arc::new(Admitted { stored: Stored { digest, reads }, data }), place: Some(placed.at) }
    }
    /// A frame of rows already encoded (`encode_rows`).
    pub fn encoded_as(rel: Rel, part: i64, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, n: usize, bytes: Vec<u8>) -> Encoded {
        Encoded { rel, part, frame: Arc::new(Admitted { stored: Stored { digest, reads }, data: Data::Encoded { types, n, bytes: Bytes::Heap(Arc::new(bytes)), store: None } }), place: None }
    }
}

/// A write's frames encoded where they are stored (decision 0295 §1, Q93): its segments' writer, shared by the jobs
/// that make the frames, each frame's bytes written into its entry as its rows are read. Made whole on the heap
/// first, every frame of a write was held at once until it was queued: the cross part's frames, the peak of a cold
/// PostHog run.
pub struct FramePlacer {
    w: std::sync::Mutex<super::segments::SegmentWriter>,
}

/// A frame's place in its write's segments (`FramePlacer::place_rows`).
pub struct Placed {
    at: usize,
}

impl FramePlacer {
    /// Encodes rows of `width` columns, given a window at a time (`each`, called twice), into the next entry, as
    /// `encode_rows` encodes them: counted outside the lock, written under it. `None` where no segment takes it
    /// (the caller encodes it on the heap as before).
    pub fn place_rows(&self, width: usize, part: i64, each: &dyn Fn(&mut dyn FnMut(&[i64]))) -> Option<Placed> {
        if width == 0 {
            return None;
        }
        let mut at = None;
        super::cross_rows::encode_windows(width, each, |len, fill| {
            let mut w = self.w.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            w.put(len, part, &[0; 32], |out| fill(out))?;
            at = Some(w.len() - 1);
            Ok::<(), super::StoreError>(())
        })
        .ok()?;
        Some(Placed { at: at? })
    }

    /// Tests: a placed frame's bytes, before its write.
    #[cfg(test)]
    pub fn placed_bytes(&self, p: &Placed) -> Option<Vec<u8>> {
        self.w.lock().unwrap().entry_bytes(p.at)
    }
}

/// An encoded frame to write (`write_frames`).
struct Framed<'a> {
    rel: &'a Rel,
    part: i64,
    stored: &'a Stored,
    types: &'a [u8],
    n: usize,
    bytes: &'a [u8],
    /// Its entry of the write's segments (`place_frames`): written as a pointer, not its bytes.
    place: Option<usize>,
}

impl Framed<'_> {
    /// Its bytes with each symbol column (`b's'`) passed through `map` (this process's ids to the store's, decision
    /// 0170); `None` where it has none.
    fn store_block(&self, mut map: impl FnMut(&mut [i64]) -> StoreResult<()>) -> StoreResult<Option<Vec<u8>>> {
        let w = self.types.len();
        if !self.types.contains(&b's') || self.n == 0 {
            return Ok(None);
        }
        let mut cols = vec![Vec::new(); w];
        super::cross_rows::decode(w, self.n, self.bytes, &mut cols)?;
        for (c, col) in cols.iter_mut().enumerate() {
            if self.types[c] == b's' {
                map(col)?;
            }
        }
        let mut rows = Vec::with_capacity(w * self.n);
        for r in 0..self.n {
            rows.extend(cols.iter().map(|c| c[r]));
        }
        Ok(Some(super::cross_rows::encode(w, &rows)))
    }
}

/// Holds the stored atoms the symbol columns of `frames` (rows read from the store) name, in one read (decision
/// 0170), as they are read before a run's waves: so the waves decode them without reading the store, and a string a
/// wave interns that is not in memory is none of theirs. Every stored atom held here already, nothing is decoded.
fn hold_symbols<'a>(atoms: &super::atoms::Atoms, frames: impl IntoIterator<Item = &'a Arc<Admitted>>) -> StoreResult<()> {
    if !atoms.holds_by_reading() {
        return Ok(());
    }
    let mut ids = Vec::new();
    for frame in frames {
        frame.stored_symbols(&mut ids)?;
    }
    ids.sort_unstable();
    ids.dedup();
    atoms.hold_named(&ids)
}

fn encode_reads(reads: &[(String, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, digest) in reads {
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(digest.to_le_bytes());
    }
    out
}

/// The space of the per-file memo of plan `plan` (decision 0146): one space per file-local program.
pub fn local_space(plan: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.space/v1\0local\0");
    h.update(plan);
    h.finalize().into()
}

/// The run of `space` that run `token` is: counted once per token (`0`: a run of its own, counted each time).
fn count_run(t: &rusqlite::Connection, space: &[u8; 32], token: u64) -> rusqlite::Result<i64> {
    t.execute_cached("INSERT INTO frame_spaces(space, runs) VALUES (?1, 0) ON CONFLICT(space) DO NOTHING", [&space[..]])?;
    let run: i64 = t.query_row_cached("SELECT runs FROM frame_spaces WHERE space = ?1", [&space[..]], |r| r.get(0))?;
    let mut counted = counted().lock().expect("the spaces' counted runs");
    if token == 0 || counted.get(space) != Some(&token) {
        t.execute_cached("UPDATE frame_spaces SET runs = runs + 1 WHERE space = ?1", [&space[..]])?;
        counted.insert(*space, token);
        return Ok(run + 1);
    }
    Ok(run)
}

/// A frame read by part: `(part, digest, reads, rows, types, data, seg, seg_at, seg_len)`; `None` where its segment is
/// not there.
fn admitted_part(row: &rusqlite::Row<'_>, atoms: &Arc<super::atoms::Atoms>, maps: &mut super::segments::SegmentMaps) -> rusqlite::Result<Option<(i64, Arc<Admitted>)>> {
    let Some(bytes) = row_bytes(maps, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?) else { return Ok(None) };
    Ok(Some((row.get(0)?, Arc::new(Admitted {
        stored: Stored { digest: row.get::<_, i64>(1)? as u64, reads: decode_reads(&row.get::<_, Vec<u8>>(2)?) },
        data: Data::Encoded { n: row.get::<_, i64>(3)? as usize, types: row.get(4)?, bytes, store: Some(Arc::clone(atoms)) },
    }))))
}

/// The entry a base frame's stored reads end with when deltas follow it (decision 0117 step 4): how many. A frame
/// put whole has none, so deltas a put left behind are not read.
const CHAIN: &str = "~deltas";

/// A frame's stored reads, and how many deltas follow it.
fn decode_reads_chain(bytes: &[u8]) -> (Vec<(String, u64)>, u32) {
    let mut reads = decode_reads_all(bytes);
    let k = match reads.last() {
        Some((name, k)) if name == CHAIN => *k as u32,
        _ => return (reads, 0),
    };
    reads.pop();
    (reads, k)
}

fn decode_reads(bytes: &[u8]) -> Vec<(String, u64)> {
    decode_reads_chain(bytes).0
}

fn decode_reads_all(mut bytes: &[u8]) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    while bytes.len() >= 4 {
        let len = u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes")) as usize;
        if bytes.len() < 4 + len + 8 {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[4..4 + len]).into_owned();
        let digest = u64::from_le_bytes(bytes[4 + len..12 + len].try_into().expect("8 bytes"));
        out.push((name, digest));
        bytes = &bytes[12 + len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn frame_publication_batches_put_delete_and_usage_with_bounded_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let puts: Vec<_> = (0..8192).map(|i| Put { rel: rel(&format!("r{i}"), 1), part: 0,
            digest: 7, reads: vec![], rows: Rows { types: vec![0], rows: vec![i], n: 1 } }).collect();
        store.keep_frames(&[1; 32], &puts, &[], None).unwrap();
        let mut changed: Vec<_> = puts.iter().step_by(31).take(257).map(|p| Put { rel: p.rel.clone(), part: 0,
            digest: 9, reads: vec![], rows: Rows { types: vec![0], rows: vec![999], n: 1 } }).collect();
        // One batch can publish the same key twice. The last publication wins, as before.
        changed.push(Put { rel: changed[0].rel.clone(), part: 0, digest: 10, reads: vec![],
            rows: Rows { types: vec![], rows: vec![], n: 1 } });
        let gone: Vec<_> = puts.iter().skip(1).step_by(31).take(257).map(|p| (p.rel.clone(),0)).collect();
        let used: Vec<_> = changed.iter().map(|p| p.rel.clone()).collect();
        let counters = [AtomicUsize::new(0),AtomicUsize::new(0)];
        unsafe extern "C" fn trace(event: u32, state: *mut std::ffi::c_void, stmt: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
            let c = unsafe { &*state.cast::<[AtomicUsize;2]>() };
            if event == rusqlite::ffi::SQLITE_TRACE_STMT {
                c[0].fetch_add(1, Ordering::Relaxed);
                // Cached statements include untraced fixture setup unless reset at execution start.
                unsafe {rusqlite::ffi::sqlite3_stmt_status(stmt.cast(),rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,1)};
            }
            else {
                let vm=unsafe {rusqlite::ffi::sqlite3_stmt_status(stmt.cast(),rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,1)} as usize;
                c[1].fetch_add(vm,Ordering::Relaxed);
            }
            0
        }
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),
            rusqlite::ffi::SQLITE_TRACE_STMT | rusqlite::ffi::SQLITE_TRACE_PROFILE, Some(trace), (&counters as *const [AtomicUsize;2]).cast_mut().cast()); }
        let result = store.keep_frames(&[1;32], &changed, &gone, Some((&used, 1, [0; 32])));
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),0,None,std::ptr::null_mut()); }
        result.unwrap();
        eprintln!("frame publication total: {} statements, {} VM",counters[0].load(Ordering::Relaxed),counters[1].load(Ordering::Relaxed));
        assert_eq!(store.frame_rows(&[1;32], &changed[0].rel,0).unwrap(),Some(changed.last().unwrap().rows.clone()));
        for (r,part) in &gone { assert!(store.frame_rows(&[1;32],r,*part).unwrap().is_none()); }
        assert!(counters[0].load(Ordering::Relaxed)<=20,"257 publications/deletions/touches must be bounded batches: {} statements",counters[0].load(Ordering::Relaxed));
        assert!(counters[1].load(Ordering::Relaxed)<100_000,"publication must probe keys instead of multiplying the space by demands: {} VM",counters[1].load(Ordering::Relaxed));
    }

    #[test]
    fn frame_admission_probes_demanded_keys_instead_of_scanning_the_space() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let puts: Vec<_> = (0..8192).map(|i| Put { rel: rel(&format!("r{i}"), 1), part: 0,
            digest: 7, reads: vec![], rows: Rows { types: vec![0], rows: vec![i], n: 1 } }).collect();
        store.keep_frames(&[1; 32], &puts, &[], None).unwrap();
        let demands: Vec<_> = puts.iter().step_by(64).map(|p| p.rel.clone()).collect();
        let work = AtomicUsize::new(0);
        unsafe extern "C" fn trace(_: u32, state: *mut std::ffi::c_void, statement: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
            let vm = unsafe { rusqlite::ffi::sqlite3_stmt_status(statement.cast(), rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP, 1) };
            unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(vm as usize, Ordering::Relaxed);
            0
        }
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),
            rusqlite::ffi::SQLITE_TRACE_PROFILE, Some(trace), (&work as *const AtomicUsize).cast_mut().cast()); }
        let got = store.admit_frames(&[1; 32], &demands, 0);
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(), 0, None, std::ptr::null_mut()); }
        let got = got.unwrap();
        assert_eq!(got.len(), 128);
        for p in puts.iter().step_by(64) { assert_eq!(got[&p.rel].rows().unwrap(), p.rows); }
        assert!(work.load(Ordering::Relaxed) < 100_000,
            "128 demands must use full-key probes, not scan 8192 stored frames: {} VM instructions", work.load(Ordering::Relaxed));
    }

    /// A frame encoded straight into the write's segments as its rows come (Q93), a window at a time: its bytes are
    /// `encode`'s, and it reads back as a frame queued whole does, queued and stored, beside one queued as before.
    #[test]
    fn a_frame_placed_as_it_is_encoded_reads_back_as_one_queued_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let (r, s) = (rel("r", 2), rel("s", 1));
        let width = 3;
        let values: Vec<i64> = (0..70_000i64).flat_map(|i| [i, -7 * i, (i % 13) << 40]).collect();
        let n = values.len() / width;
        let each = |f: &mut dyn FnMut(&[i64])| {
            for w in values.chunks(1000 * width) {
                f(w);
            }
        };
        let sorted = |mut x: Vec<i64>, w: usize| -> Vec<Vec<i64>> {
            let mut rows: Vec<Vec<i64>> = x.chunks_mut(w).map(|c| c.to_vec()).collect();
            rows.sort();
            rows
        };
        let want = sorted(values.clone(), width);
        {
            let store = ContentStore::open(&path).unwrap();
            let placer = store.frame_placer(2, 1 << 16);
            let placed = placer.place_rows(width, 0, &each).expect("placed");
            assert_eq!(placer.placed_bytes(&placed).as_deref(), Some(&super::super::cross_rows::encode(width, &values)[..]), "encode's bytes");
            let frames = vec![
                Put::placed_as(r.clone(), 0, 7, vec![("a".into(), 1)], vec![b'i'; width], n, placed),
                Put { rel: s.clone(), part: 0, digest: 8, reads: vec![], rows: Rows { types: vec![b'i'; 2], rows: vec![1, 2, 3, 4], n: 2 } }.encoded(),
            ];
            store.queue_placed_retagged([1; 32], frames, vec![], vec![], vec![], None, placer);
            let now = store.admit_frames(&[1; 32], &[r.clone(), s.clone()], 0).unwrap();
            assert_eq!(now[&r].stored, Stored { digest: 7, reads: vec![("a".into(), 1)] });
            assert_eq!(sorted(now[&r].rows().unwrap().rows, width), want, "queued, in memory");
            assert_eq!(sorted(now[&s].rows().unwrap().rows, 2), vec![vec![1, 2], vec![3, 4]]);
        }
        let store = ContentStore::open(&path).unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone(), s.clone()], 0).unwrap();
        assert_eq!(sorted(got[&r].rows().unwrap().rows, width), want, "stored");
        assert_eq!(sorted(got[&s].rows().unwrap().rows, 2), vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn a_relations_deltas_are_read_after_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let r = rel("r", 1);
        let rows = |v: &[i64]| Rows { types: vec![0, 0], rows: v.to_vec(), n: v.len() / 2 };
        let sorted = |x: Vec<i64>| { let mut p: Vec<[i64; 2]> = x.chunks(2).map(|c| [c[0], c[1]]).collect(); p.sort(); p };
        let delta = |digest, gone: &[i64], added: &[i64]| Delta { rel: rel("r", 1), digest, reads: vec![("a".into(), digest)], types: vec![0, 0], gone: gone.to_vec(), added: added.to_vec() };
        {
            let store = ContentStore::open(&path).unwrap();
            store.queue_frames([1; 32], vec![Put { rel: r.clone(), part: 0, digest: 1, reads: vec![], rows: rows(&[1, 1, 2, 2, 3, 3]) }], vec![], None);
            store.queue_frames_retagged([1; 32], vec![], vec![], vec![delta(2, &[2, 2], &[4, 4])], vec![], None);
            // Queued, in memory: the rows after the delta.
            let now = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
            assert_eq!(now[&r].stored.digest, 2);
            assert_eq!(sorted(now[&r].rows().unwrap().rows), vec![[1, 1], [3, 3], [4, 4]]);
            assert_eq!(now[&r].chain(), (1, 2));
            store.queue_frames_retagged([1; 32], vec![], vec![], vec![delta(3, &[1, 1], &[5, 5])], vec![], None);
        }
        // Stored: read from SQL with its deltas, oldest first.
        let store = ContentStore::open(&path).unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        assert_eq!(got[&r].stored, Stored { digest: 3, reads: vec![("a".into(), 3)] });
        assert_eq!(sorted(got[&r].rows().unwrap().rows), vec![[3, 3], [4, 4], [5, 5]]);
        assert_eq!(got[&r].chain(), (2, 4));
        // A frame retagged keeps its rows, deltas and stored reads; the space's confirmations name its reads now
        // (decision 0117 step 5).
        store.queue_frames_retagged([1; 32], vec![], vec![Retag { rel: r.clone(), part: 0, reads: vec![("b".into(), 4)] }], vec![], vec![], None);
        drop(store);
        let store = ContentStore::open(&path).unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        assert_eq!((got[&r].stored.reads.clone(), got[&r].chain()), (vec![("a".into(), 3)], (2, 4)));
        assert_eq!(store.confirmed(&[1; 32]).unwrap().get(&confirm_key(&r, 0)), Some(&reads_hash(&[("b".into(), 4)])));
        assert_eq!(sorted(got[&r].rows().unwrap().rows), vec![[3, 3], [4, 4], [5, 5]]);
        drop(store);
        // And from the frames read before the first wave.
        let store = ContentStore::open(&path).unwrap();
        store.preload_frames().unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        assert_eq!(sorted(got[&r].rows().unwrap().rows), vec![[3, 3], [4, 4], [5, 5]]);
        assert_eq!(got[&r].chain(), (2, 4));
        // A frame put whole ends its deltas.
        store.keep_frames(&[1; 32], &[Put { rel: r.clone(), part: 0, digest: 9, reads: vec![], rows: rows(&[9, 9]) }], &[], None).unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        assert_eq!((sorted(got[&r].rows().unwrap().rows), got[&r].chain()), (vec![[9, 9]], (0, 0)));
        // A frame put whole is no longer confirmed: in memory (the snapshot's), and in the store.
        assert_eq!(store.confirmed(&[1; 32]).unwrap().get(&confirm_key(&r, 0)), None);
        forget_snapshot(path.to_str().unwrap());
        assert_eq!(store.confirmed(&[1; 32]).unwrap().get(&confirm_key(&r, 0)), None);
        let left: i64 = store.connection.lock().unwrap().query_row("SELECT count(*) FROM frames WHERE part < 0", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        forget_snapshot(path.to_str().unwrap());
    }

    #[test]
    fn mixed_partition_admission_uses_bounded_full_key_probes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let puts: Vec<_> = (0..8192).map(|i| Put {
            rel: rel(&format!("r{}", i % 32), 1), part: i / 32,
            digest: i as u64, reads: vec![], rows: Rows {types:vec![b'i'],n:1,rows:vec![i]}
        }).collect();
        store.keep_frames(&[1;32], &puts, &[], None).unwrap();
        let mut keys: Vec<_> = puts.iter().step_by(31).map(|p|(p.rel.clone(),p.part)).collect();
        let unique = keys.len();
        keys.extend(keys.clone());
        keys.push((rel("r0", 2), 0)); // Same name, different identity: absent.
        let counters = [AtomicUsize::new(0), AtomicUsize::new(0)];
        unsafe extern "C" fn trace(_:u32,state:*mut std::ffi::c_void,statement:*mut std::ffi::c_void,_:*mut std::ffi::c_void)->i32 {
            let c=unsafe {&*state.cast::<[AtomicUsize;2]>()};
            c[0].fetch_add(1,Ordering::Relaxed);
            let vm=unsafe {rusqlite::ffi::sqlite3_stmt_status(statement.cast(),rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,1)};
            c[1].fetch_add(vm as usize,Ordering::Relaxed);0
        }
        unsafe {rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),rusqlite::ffi::SQLITE_TRACE_PROFILE,
            Some(trace),(&counters as *const [AtomicUsize;2]).cast_mut().cast());}
        let got=store.admit_frame_parts(&[1;32],&keys);
        unsafe {rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),0,None,std::ptr::null_mut());}
        let got=got.unwrap();
        assert_eq!(got.len(),unique);
        for p in puts.iter().step_by(31) {assert_eq!(got[&(p.rel.clone(),p.part)].rows().unwrap(),p.rows);}
        assert!(store.admit_frame_parts(&[2;32],&keys).unwrap().is_empty());
        let sql=counters[0].load(Ordering::Relaxed);let vm=counters[1].load(Ordering::Relaxed);
        eprintln!("{unique} mixed-part demands: {sql} SQL, {vm} VM instructions");
        assert!(sql<=4,"one schema check and three batches, not one read per part: {sql}");
        assert!(vm<100_000,"demands must probe full keys, not scan 8192 frames: {vm}");
    }

    #[test]
    fn frame_metadata_reads_are_batched() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let rels: Vec<_> = (0..257).map(|i| rel(&format!("r{i}"), 1)).collect();
        let puts: Vec<_> = rels.iter().map(|r| Put { rel: r.clone(), part: 0, digest: 7,
            reads: vec![], rows: Rows { types: vec![0], rows: vec![42], n: 1 } }).collect();
        store.keep_frames(&[1; 32], &puts, &[], None).unwrap();
        let count = AtomicUsize::new(0);
        unsafe extern "C" fn trace(_: u32, state: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
            unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
            0
        }
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),
            rusqlite::ffi::SQLITE_TRACE_STMT, Some(trace), (&count as *const AtomicUsize).cast_mut().cast()); }
        let got = store.frames(&[1; 32], &rels, 0);
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(), 0, None, std::ptr::null_mut()); }
        assert_eq!(got.unwrap().len(), 257);
        assert!(count.load(Ordering::Relaxed) <= 4, "257 relations must use at most three batches plus schema admission, got {}", count.load(Ordering::Relaxed));
    }

    /// A frame is queued encoded, as the store keeps it, its bytes placed in a segment (decision 0295 §4): the queue
    /// counts what it holds in memory, not those bytes (the OS's pages; before, its encoded bytes, and the raw rows
    /// were five times those), and it reads the same before and after the writer stores it.
    #[test]
    fn frames_are_queued_encoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let r = rel("span_within", 1);
        let values: Vec<i64> = (0..20_000i64).flat_map(|i| [(7 << 32) | (i + 1), (7 << 32) | (i / 4)]).collect();
        let put = || Put { rel: r.clone(), part: 3, digest: 9, reads: vec![], rows: Rows { types: vec![b'i', b'i'], rows: values.clone(), n: 20_000 } };
        let encoded = super::super::cross_rows::encode(2, &values).len();
        {
            let store = ContentStore::open(&path).unwrap();
            let (release, blocked) = std::sync::mpsc::channel::<()>();
            store.write_back("test gate", 0, vec![], crate::content_store::write_back::Work::test(Box::new(move |_| { blocked.recv().unwrap(); Ok(()) })));
            store.queue_frames([1; 32], vec![put()], vec![], None);
            let queued = store.admit_frames(&[1; 32], &[r.clone()], 3).unwrap();
            assert_eq!(queued[&r].rows().unwrap().rows, values, "queued: read from its encoded bytes");
            assert_eq!(queued[&r].encoded_bytes(), encoded);
            release.send(()).unwrap();
            store.written().unwrap();
            let (by, _, _) = store.written_by_what();
            let frames = by.iter().find(|w| w.0 == "frames").expect("the frames' job");
            assert!(frames.2 < encoded / 100, "the queue counts its record, not the {encoded} bytes placed: {}", frames.2);
            assert!(encoded * 4 < values.len() * 8, "{encoded} encoded of {} raw", values.len() * 8);
        }
        let store = ContentStore::open(&path).unwrap();
        assert_eq!(store.frame_rows(&[1; 32], &r, 3).unwrap().unwrap().rows, values, "stored: the same rows");
    }

    #[test]
    fn queued_frames_are_read_without_waiting_for_the_writer() {
        use std::sync::mpsc;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let store = ContentStore::open(&path).unwrap();
        let reader = ContentStore::open(&path).unwrap();
        let (release, blocked) = mpsc::channel();
        store.write_back("test gate", 0, vec![], crate::content_store::write_back::Work::test(Box::new(move |_| { blocked.recv().unwrap(); Ok(()) })));
        let r = rel("empty", 1);
        store.queue_frames([1; 32], vec![Put { rel: r.clone(), part: 0, digest: 7,
            reads: vec![], rows: Rows { types: vec![0], rows: vec![], n: 0 } }], vec![], None);
        let (tx, rx) = mpsc::channel();
        let read = std::thread::spawn(move || {
            let meta = reader.frames(&[1; 32], &[r.clone()], 0).unwrap();
            let rows = reader.frame_rows(&[1; 32], &r, 0).unwrap();
            tx.send((meta, rows)).unwrap();
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(2));
        // Always unblock the writer, including when the old reader waits for it.
        release.send(()).unwrap();
        read.join().unwrap();
        let (meta, rows) = got.expect("resident queued facts must not wait for SQLite");
        assert_eq!(meta[&rel("empty", 1)].digest, 7);
        assert_eq!(rows.unwrap().n, 0, "an empty answer is a completed relation");
    }

    #[test]
    fn admitted_frames_survive_store_close_and_preserve_identity_partition_and_empty_answers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let r = rel("r", 1);
        let empty = rel("empty", 1);
        let nullary = rel("unit", 1);
        let put = |r: &Rel, part, rows| Put { rel: r.clone(), part, digest: 7, reads: vec![("input".into(), 9)], rows };
        {
            let store = ContentStore::open(&path).unwrap();
            store.queue_frames([1; 32], vec![
                put(&r, 0, Rows { types: vec![0, 1], rows: vec![-9, 1 << 40, 2, 3], n: 2 }),
                put(&r, 1, Rows { types: vec![0], rows: vec![99], n: 1 }),
                put(&empty, 0, Rows { types: vec![0], rows: vec![], n: 0 }),
                put(&nullary, 0, Rows { types: vec![], rows: vec![], n: 1 }),
            ], vec![], None);
        }
        let store = ContentStore::open(&path).unwrap();
        let frames = store.admit_frames(&[1; 32], &[r.clone(), r.clone(), rel("r", 2), empty.clone(), nullary.clone()], 0).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(store.admit_frames(&[1; 32], &[r.clone()], 1).unwrap()[&r].rows().unwrap().rows, [99]);
        assert!(store.admit_frames(&[2; 32], &[r.clone()], 0).unwrap().is_empty());
        drop(store);
        // No open store or connection remains: every consumer uses admitted memory.
        assert_eq!(frames[&r].rows().unwrap().rows, [-9, 1 << 40, 2, 3]);
        assert_eq!(frames[&empty].rows().unwrap().n, 0);
        assert_eq!(frames[&nullary].rows().unwrap().n, 1);
        assert_eq!(frames[&r].stored.reads, [("input".to_owned(), 9)]);
    }

    #[test]
    fn queued_replacement_and_deletion_override_durable_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let store = ContentStore::open(&path).unwrap();
        let reader = ContentStore::open(&path).unwrap();
        let r = rel("r", 1);
        let put = |value| Put { rel: r.clone(), part: 0, digest: value as u64, reads: vec![],
            rows: Rows { types: vec![0], rows: vec![value], n: 1 } };
        store.keep_frames(&[1; 32], &[put(1)], &[], None).unwrap();
        let (release, blocked) = std::sync::mpsc::channel();
        store.write_back("test gate", 0, vec![], crate::content_store::write_back::Work::test(Box::new(move |_| { blocked.recv().unwrap(); Ok(()) })));
        store.queue_frames([1; 32], vec![put(2)], vec![], None);
        let replaced = reader.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        store.queue_frames([1; 32], vec![], vec![(r.clone(), 0)], None);
        let deleted = reader.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        store.queue_frames([1; 32], vec![put(3)], vec![], None);
        let latest = reader.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        release.send(()).unwrap();
        store.written().unwrap();
        assert_eq!(replaced[&r].rows().unwrap().rows, [2]);
        assert!(deleted.is_empty());
        assert_eq!(latest[&r].rows().unwrap().rows, [3]);
        assert_eq!(reader.frame_rows(&[1; 32], &r, 0).unwrap().unwrap().rows, [3]);
    }

    /// Decision 0227: what is kept is what the kept manifests reach, with no run's marks. A memo entry stays while its
    /// plan is live and a kept manifest names its content; a maintained relation while a live program (a plan a
    /// manifest names) derives it in its space. A program's second write records nothing new.
    #[test]
    fn frames_are_kept_while_a_kept_manifest_reaches_them() {
        use sha2::{Digest as _, Sha256};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let (plan, program, gone_program) = ([5u8; 32], [7u8; 32], [8u8; 32]);
        let text = b"x = 1\n";
        let source = store.put_source(text, &[]).unwrap();
        // A run of root /r, which read `source` and names `program` (a part's key) among its plans.
        store.record_corpus_run_naming("/r", &[source], 1, 1, plan, &[program], 1).unwrap();
        let memo = |path: &str, content: &[u8]| MemoPut { part: path.len() as i64, path: path.into(), digest: Sha256::digest(content).into(), key: 1, bytes: vec![1, 2, 3], over: false };
        let space = local_space(&plan);
        store.put_memo_entries(&space, vec![memo("a.py", text), memo("old.py", b"y = 2\n")], 4).unwrap();
        let maintained = [9u8; 32];
        let (kept, dropped) = (rel("kept", 1), rel("dropped", 1));
        let put = |r: &Rel| Put { rel: r.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        store.keep_frames(&maintained, &[put(&kept)], &[], Some((&[kept.clone()][..], 0, program))).unwrap();
        store.keep_frames(&maintained, &[put(&dropped)], &[], Some((&[dropped.clone()][..], 0, gone_program))).unwrap();
        let rows = |sql: &str| -> i64 { store.connection.lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap() };
        let used = rows("SELECT total_changes()");
        store.keep_frames(&maintained, &[], &[], Some((&[kept.clone()][..], 0, program))).unwrap();
        assert_eq!(rows("SELECT total_changes()") - used, 1, "a program's relations again: only the space's count moves, nothing of its use");
        let swept = {
            let c = store.connection.lock().unwrap();
            collect_unreachable_frames(&c).unwrap()
        };
        assert_eq!(swept, Unreachable { memo: 1, maintained: 1, programs: 1, unnamed: 1 }, "the gone program no manifest names");
        let found = store.memo_entries(&space, &["a.py".len() as i64, "old.py".len() as i64], true).unwrap();
        assert!(found.contains_key(&("a.py".len() as i64)), "a kept manifest's content");
        assert!(!found.contains_key(&("old.py".len() as i64)), "content no manifest names");
        assert_eq!(found[&("a.py".len() as i64)].bytes.bytes(), &[1, 2, 3], "its bytes, from its segment");
        let got = store.frames(&maintained, &[kept.clone(), dropped.clone()], 0).unwrap();
        assert!(got.contains_key(&kept) && !got.contains_key(&dropped), "a live program's relation stays, a dead one's goes");
        let again = {
            let c = store.connection.lock().unwrap();
            collect_unreachable_frames(&c).unwrap()
        };
        assert!(!again.any(), "nothing left to collect: {again:?}");
    }

    /// Decision 0302: an edited program reuses its predecessor's frames and puts nothing (decision 0227: a warm run
    /// writes nothing). Its predecessor superseded and dead, only its own holding keeps them: `hold_frames` records
    /// it, queued once, and only when the store has none (a run of a program that holds queues nothing, 0197).
    #[test]
    fn a_program_that_reuses_frames_holds_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let (old, new) = ([7u8; 32], [8u8; 32]);
        let maintained = [9u8; 32];
        let kept = rel("kept", 1);
        {
            let store = ContentStore::open(&path).unwrap();
            let source = store.put_source(b"x = 1\n", &[]).unwrap();
            // The edited run's manifest names `new`; `old`, its predecessor, no manifest names.
            store.record_corpus_run_naming("/r", &[source], 1, 1, [5u8; 32], &[new], 1).unwrap();
            let put = Put { rel: kept.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
            store.keep_frames(&maintained, &[put], &[], Some((&[kept.clone()][..], 0, old))).unwrap();
            // The run's read before its first wave (decision 0104).
            store.preload_frames_live(None).unwrap();
            assert!(store.hold_frames(maintained, vec![kept.clone()], 0, new), "no holding: queued");
            assert!(!store.hold_frames(maintained, vec![kept.clone()], 0, new), "held, this process: nothing queued");
            assert!(!store.hold_frames(maintained, vec![kept.clone()], 0, old), "the program that put the frame holds them");
            store.written().unwrap();
        }
        forget_snapshot(path.to_str().unwrap());
        let store = ContentStore::open(&path).unwrap();
        store.preload_frames_live(None).unwrap();
        assert!(!store.hold_frames(maintained, vec![kept.clone()], 0, new), "held, in the store: nothing queued");
        let swept = {
            let c = store.connection.lock().unwrap();
            collect_unreachable_frames(&c).unwrap()
        };
        assert_eq!(swept.maintained, 0, "the reusing program holds the frame: {swept:?}");
        assert!(store.frames(&maintained, &[kept.clone()], 0).unwrap().contains_key(&kept));
    }

    fn rel(name: &str, i: u8) -> Rel {
        Rel { name: name.into(), ident: [i; 32] }
    }

    /// Decision 0295 §4: a queued frame's bytes are placed in a segment by the run and read there; its written row
    /// points at them (no bytes of its own), and a read from the store maps them. Written again, its old bytes are
    /// no row's, and compaction leaves one segment of the named ones.
    #[test]
    fn a_queued_frame_is_placed_in_a_segment_and_read_there() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let (space, r) = ([4u8; 32], rel("r", 1));
        let put = |rows: Vec<i64>| Put { rel: r.clone(), part: 0, digest: 9, reads: vec![("input".into(), 4)], rows: Rows { types: vec![0, 1], n: rows.len() / 2, rows } };
        let from_file = |store: &ContentStore| store.read_frame_parts_from(&space, &[(r.clone(), 0)], true, false).unwrap().remove(&(r.clone(), 0)).expect("read from the store").rows().unwrap();
        store.queue_frames(space, vec![put(vec![5, -3, 1 << 40, 7])], Vec::new(), None);
        store.written().unwrap();
        let (seg, data): (i64, i64) = store.connection.lock().unwrap().query_row("SELECT seg, length(data) FROM frames", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert!(seg > 0 && data == 0, "a pointer, no bytes: {seg} {data}");
        assert_eq!(from_file(&store).rows, [5, -3, 1 << 40, 7]);
        let (held, named) = store.frame_segment_bytes().unwrap();
        assert!(held > named && named > 0, "{held} {named}");
        // Written again: the first bytes are no row's.
        let big: Vec<i64> = (0..20_000).collect();
        store.queue_frames(space, vec![put(vec![1; 40_000])], Vec::new(), None);
        store.queue_frames(space, vec![put(big.clone())], Vec::new(), None);
        store.written().unwrap();
        assert!(store.compact_frame_segments(0).unwrap() > 0);
        assert_eq!(std::fs::read_dir(frame_dir(&store.database_path)).unwrap().count(), 1, "one segment left");
        assert_eq!(from_file(&store).rows, big, "read from the new segment");
        assert_eq!(store.frame_rows(&space, &r, 0).unwrap().unwrap().rows, big, "and from the snapshot");
    }

    #[test]
    fn a_frame_is_read_back_with_what_it_was_derived_from() {
        let dir = std::env::temp_dir().join(format!("rerail-frames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let space = [1u8; 32];
        let (r, s) = (rel("r", 1), rel("r", 2));
        assert!(store.frames(&space, &[r.clone()], 0).unwrap().is_empty());
        let rows = Rows { types: vec![0, 1], rows: vec![5, -3, 1 << 40, 7], n: 2 };
        let put = Put { rel: r.clone(), part: 0, digest: 9, reads: vec![("input".into(), 4), ("q".into(), 1 << 63)], rows };
        store.keep_frames(&space, &[put], &[], None).unwrap();
        let got = store.frames(&space, &[r.clone(), s.clone()], 0).unwrap();
        assert_eq!(got.get(&r), Some(&Stored { digest: 9, reads: vec![("input".into(), 4), ("q".into(), 1 << 63)] }));
        // Another identity of the same name is another relation.
        assert_eq!(got.get(&s), None);
        let back = store.frame_rows(&space, &r, 0).unwrap().unwrap();
        assert_eq!((back.types, back.rows, back.n), (vec![0, 1], vec![5, -3, 1 << 40, 7], 2));
        // Replaced, then forgotten.
        let put = Put { rel: r.clone(), part: 0, digest: 10, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        store.keep_frames(&space, &[put], &[], None).unwrap();
        assert_eq!(store.frames(&space, &[r.clone()], 0).unwrap()[&r].digest, 10);
        store.keep_frames(&space, &[], &[(r.clone(), 0)], None).unwrap();
        assert!(store.frames(&space, &[r.clone()], 0).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decision 0117: a memo keeps a path's newest contents only. Decision 0240 §4: its bytes are in the pack, which
    /// compaction rewrites with only what its rows name; and the frames read before the first wave hold none of it.
    #[test]
    fn a_memo_keeps_a_paths_newest_and_its_pack_is_compacted() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let space = local_space(&[5u8; 32]);
        // Entries of 4000 bytes: a segment's header and entry table are small beside them.
        const N: u64 = 4000;
        let put = |part: i64, path: &str| MemoPut { part, path: path.into(), digest: [part as u8; 32], key: 1, bytes: vec![part as u8; N as usize], over: false };
        let segments = || std::fs::read_dir(dir.path().join("store.sqlite.memo")).unwrap().count();
        // Six contents of a.py, one batch each, and one of b.py.
        for part in 0..6 {
            store.put_memo_entries(&space, vec![put(part, "a.py")], 4).unwrap();
        }
        store.put_memo_entries(&space, vec![put(100, "b.py")], 4).unwrap();
        let parts: Vec<i64> = store.connection.lock().unwrap().prepare("SELECT part FROM memo_entries WHERE space = ?1 ORDER BY part").unwrap().query_map([&space[..]], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(parts, vec![2, 3, 4, 5, 100], "a.py's two oldest contents go");
        let (held, named) = store.memo_pack_bytes().unwrap();
        assert_eq!((named, segments()), (5 * N, 7), "one segment a write");
        assert!(held > 7 * N && held < 7 * N + 7 * 1024, "each segment its entry and a little more: {held}");
        assert_eq!(store.compact_memo_pack(1 << 40).unwrap(), 0, "below the floor");
        assert_eq!(store.compact_memo_pack(0).unwrap(), 0, "less than half unnamed");
        store.put_memo_entries(&space, (0..6).map(|i| put(200 + i, "c.py")).collect(), 1).unwrap();
        assert_eq!(store.memo_pack_bytes().unwrap().1, 6 * N);
        let before = store.memo_entries(&space, &[2, 5, 100, 205], true).unwrap();
        let (held, _) = store.memo_pack_bytes().unwrap();
        let freed = store.compact_memo_pack(0).unwrap();
        let (now, named) = store.memo_pack_bytes().unwrap();
        assert_eq!((freed, named, segments()), (held - now, 6 * N, 1), "one segment left, of the named entries");
        assert!(now < 6 * N + 1024, "{now}");
        let after = store.memo_entries(&space, &[2, 5, 100, 205], true).unwrap();
        assert_eq!(after.len(), 4);
        for (part, blob) in &before {
            assert_eq!(after[part].bytes.bytes(), blob.bytes.bytes(), "part {part} reads the same from the new segment");
        }
        drop(before);
        let (frames, _) = store.preload_frames_live(None).unwrap();
        assert_eq!(frames, 0, "no memo entry is a frame");
    }

    /// Q19: a new process reads before its run's first wave, of its cross part's space, the frames of the identities
    /// its programs can read (every part), not another identity's (an earlier build's, an earlier edit's); every
    /// other space whole. Whichever run reads first (phase 0's, a root of its own, on a rule edit), the store's first
    /// read leaves out every recorded root's cross space.
    ///
    /// Each read is under a span of its own, so the profile attributes it (it was the `parse` phase's own time: 2 s of
    /// an edit, ep2).
    #[test]
    fn a_run_reads_its_cross_space_by_identity_whichever_run_reads_first() {
        use rerail_prof::code_rule_dsl::logic::profile;
        let _run = profile::one_run();
        profile::reset();
        profile::set(true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let cross = [9u8; 32];
        let put = |rel: &Rel, v: i64| Put { rel: rel.clone(), part: 0, digest: v as u64, reads: vec![("-".into(), 0)], rows: Rows { types: vec![0], rows: vec![v], n: 1 } };
        let (kept, stale, elsewhere) = (rel("x", 1), rel("x", 2), [11u8; 32]);
        {
            let store = ContentStore::open(&path).unwrap();
            let granule = Put { part: 3, ..put(&kept, 11) };
            store.keep_frames(&cross, &[put(&kept, 10), granule, put(&stale, 20)], &[], None).unwrap();
            store.keep_frames(&elsewhere, &[put(&stale, 30)], &[], None).unwrap();
            store.connection.lock().unwrap().execute("INSERT INTO corpus_roots(path) VALUES ('/corpus'), ('/rules')", []).unwrap();
        }
        let store = ContentStore::open(&path).unwrap();
        let rels = HashSet::from([kept.clone(), rel("y", 3)]);
        let held = |space: &[u8; 32], rel: &Rel, part: i64| store.admit_frames(space, std::slice::from_ref(rel), part).unwrap().get(rel).map(|f| f.rows().unwrap().rows);
        fn cross_of(root: &std::path::Path) -> [u8; 32] {
            if root == std::path::Path::new("/corpus") { [9; 32] } else { [12; 32] }
        }
        // A phase's run first (phase 0, over the rule files: a root of its own, before the corpus run starts).
        let phase = CrossSpace { space: [12; 32], own: true, rels: None, cross_of };
        assert_eq!(store.preload_frames_live(Some(phase)).unwrap().0, 1, "the other space's frame");
        assert_eq!(held(&cross, &kept, 0), None, "the corpus root's cross space is not read by a phase's run");
        assert_eq!(held(&elsewhere, &stale, 0), Some(vec![30]), "another space is read whole");
        // Then the run of no phase: of its cross space, its identities' frames, every part.
        let own = CrossSpace { space: cross, own: true, rels: Some(&rels), cross_of };
        assert_eq!(store.preload_frames_live(Some(own)).unwrap().0, 2, "both parts of the kept identity");
        assert_eq!((held(&cross, &kept, 0), held(&cross, &kept, 3)), (Some(vec![10]), Some(vec![11])));
        assert_eq!(held(&cross, &stale, 0), None, "an identity no program of the run reads is not read");
        assert_eq!(store.preload_frames_live(Some(own)).unwrap().0, 0, "once a process");
        let spans: Vec<String> = profile::merged().spans.keys().cloned().collect();
        profile::set(false);
        for read in ["the store's frames, read once a process", "the cross part's space, by identity (Q19)"] {
            assert!(spans.iter().any(|s| s == read), "{read}: {spans:?}");
            assert!(spans.iter().any(|s| *s == format!("{read}/their symbols held")), "{read}: {spans:?}");
        }
    }

    /// Q19: the read of a run's cross space reads the blobs of the frames it keeps, not those of every frame of the
    /// space. A space holding an earlier run's big frame (2 MB of rows, about 500 overflow pages) costs the read its
    /// keys alone: on ep2 the scan read 3.45 GB of blobs to keep 347 MB.
    ///
    /// Ignored: no read of this table does it. Its rows are its key records (`WITHOUT ROWID`), so a seek or a range
    /// scan by `space` compares whole records, blobs and all, past their first page; a lookup by key reads its
    /// neighbours' (ep2: 0.12 s to scan the space's keys by `frames_by_digest`, about 115 µs a lookup by key). The
    /// blobs leave the key's b-tree first (work queue, Q19).
    #[test]
    #[ignore = "Q19: frames' blobs are in the key's b-tree; every seek reads them"]
    fn a_cross_space_read_reads_no_blob_of_a_frame_it_leaves_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let cross = [9u8; 32];
        let (kept, stale) = (rel("x", 1), rel("x", 2));
        let overflow: Vec<i64> = {
            let store = ContentStore::open(&path).unwrap();
            let small = Put { rel: kept.clone(), part: 0, digest: 1, reads: vec![("-".into(), 0)], rows: Rows { types: vec![0], rows: vec![7], n: 1 } };
            let big: Vec<i64> = (0..250_000u64).map(|i| (i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 1) as i64).collect();
            let old = Put { rel: stale.clone(), part: 0, digest: 2, reads: vec![("-".into(), 0)], rows: Rows { types: vec![0], n: big.len(), rows: big } };
            store.keep_frames(&cross, &[small, old], &[], None).unwrap();
            let c = store.connection.lock().unwrap();
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
            // Only the stale frame's blob overflows its page.
            let mut st = c.prepare("SELECT pageno FROM dbstat WHERE name = 'frames' AND pagetype = 'overflow' ORDER BY pageno").unwrap();
            st.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
        };
        assert!(overflow.len() > 100, "the stale frame's blob overflows: {} pages", overflow.len());
        // Its overflow chain, broken: a page in its middle points past the file's end.
        let mut file = std::fs::read(&path).unwrap();
        let page = (overflow[overflow.len() / 2] as usize - 1) * 4096;
        file[page..page + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        std::fs::write(&path, &file).unwrap();
        fn cross_of(_: &std::path::Path) -> [u8; 32] {
            [9; 32]
        }
        let store = ContentStore::open(&path).unwrap();
        let whole = store.connection.lock().unwrap().query_row("SELECT sum(instr(data, x'00')) FROM frames", [], |r| r.get::<_, i64>(0)).map(|_| ());
        assert!(whole.is_err(), "reading the stale blob fails");
        let rels = HashSet::from([kept.clone()]);
        let own = CrossSpace { space: cross, own: true, rels: Some(&rels), cross_of };
        assert_eq!(store.preload_frames_live(Some(own)).unwrap().0, 1, "the kept frame, read without the stale one's blob");
        let held = store.admit_frames(&cross, std::slice::from_ref(&kept), 0).unwrap();
        assert_eq!(held[&kept].rows().unwrap().rows, vec![7]);
    }

    /// Decision 0280: what a later run keeps over an entry (its identities) is read with it, from the store and from
    /// the snapshot, survives compaction, is dropped where the entry is not the same content, and goes when the
    /// entry is written again whole.
    #[test]
    fn what_is_kept_over_an_entry_is_read_with_it_and_goes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let space = local_space(&[6u8; 32]);
        let put = |part: i64, bytes: Vec<u8>, over: bool| MemoPut { part, path: "a.py".into(), digest: [1; 32], key: 1, bytes, over };
        // Written twice whole: the first copy's bytes are no row's.
        store.put_memo_entries(&space, vec![put(1, vec![6; 4000], false)], 4).unwrap();
        store.put_memo_entries(&space, vec![put(1, vec![7; 50], false)], 4).unwrap();
        store.put_memo_entries(&space, vec![put(1, vec![8; 10], true), MemoPut { digest: [2; 32], ..put(2, vec![9; 10], true) }], 4).unwrap();
        let found = store.memo_entries(&space, &[1, 2], true).unwrap();
        assert_eq!(found.len(), 1, "an entry kept over nothing is no entry");
        let over = found[&1].over.clone().expect("kept over the entry");
        assert_eq!((found[&1].bytes.bytes(), over.bytes()), (&[7; 50][..], &[8; 10][..]));
        assert_eq!(store.memo_pack_bytes().unwrap().1, 60, "what is kept over it is named; the other is not");
        assert!(store.compact_memo_pack(0).unwrap() > 4000);
        let after = store.memo_entries(&space, &[1], true).unwrap();
        assert_eq!((after[&1].bytes.bytes(), after[&1].over.as_ref().map(|o| o.bytes())), (&[7; 50][..], Some(&[8; 10][..])), "moved together");
        store.put_memo_entries(&space, vec![put(1, vec![5; 20], false)], 4).unwrap();
        assert!(store.memo_entries(&space, &[1], true).unwrap()[&1].over.is_none(), "written again whole");
    }
}

/// A store created at `path` (its schema new): a snapshot of an earlier file there is not this one's.
pub fn forget_snapshot(path: &str) {
    snapshots().lock().expect("the frames' snapshots").remove(&super::atoms::key(std::path::Path::new(path)));
}
