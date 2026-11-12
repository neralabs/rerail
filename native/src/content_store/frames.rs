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
//! Frames are written only by the store's writer (decision 0102), behind the run. What a space keeps is the
//! relations its runs use: a relation is collected by the [`RUNS`]th run of its space in a row that does not use it.
//! A run is the outermost run (its token): the programs of one run that keep frames in one space (a phase's, P3's
//! cross part, its loading cone) each mark what they used, and the space counts the run once.

use super::{ContentStore, StoreResult};
use rusqlite::params;
use std::collections::HashMap;
use std::sync::Arc;

/// How many runs of a space in a row may leave a relation unused before it is collected.
pub const RUNS: i64 = 4;

/// The run each space last counted in this process, by its token: a later program of that run marks what it used and
/// counts nothing. A new process counts its first run of a space, since its tokens are new.
fn counted() -> &'static std::sync::Mutex<HashMap<[u8; 32], u64>> {
    static COUNTED: std::sync::OnceLock<std::sync::Mutex<HashMap<[u8; 32], u64>>> = std::sync::OnceLock::new();
    COUNTED.get_or_init(Default::default)
}

/// What a run used of a space: the relations, and the run's token (`0`: a run of its own, counted each time).
pub type Used = (Vec<Rel>, u64);

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS frames(space BLOB NOT NULL, rel TEXT NOT NULL, ident BLOB NOT NULL, \
     part INTEGER NOT NULL, digest INTEGER NOT NULL, reads BLOB NOT NULL, width INTEGER NOT NULL, rows INTEGER NOT NULL, \
     types BLOB NOT NULL, data BLOB NOT NULL, used INTEGER NOT NULL, PRIMARY KEY (space, rel, ident, part)) WITHOUT ROWID; \
     CREATE TABLE IF NOT EXISTS frame_spaces(space BLOB PRIMARY KEY, runs INTEGER NOT NULL) WITHOUT ROWID;";

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
/// Queued rows share the writer's allocation. This value belongs to the run, not a separate cache.
pub struct Admitted {
    pub stored: Stored,
    data: Data,
}

#[derive(Clone)]
enum Data {
    Encoded { types: Vec<u8>, n: usize, bytes: Arc<Vec<u8>> },
    Queued(Arc<Put>),
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
        Data::Queued(p) => Ok(p.rows.clone()),
        Data::Encoded { types, n, bytes } => {
            let width = types.len();
            let mut cols = vec![Vec::new(); width];
            super::cross_rows::decode(width, *n, bytes, &mut cols)?;
            let mut rows = Vec::with_capacity(width * n);
            for r in 0..if width == 0 { 0 } else { *n } {
                rows.extend(cols.iter().map(|c| c[r]));
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
            Data::Chain { .. } => rows_of(&self.data),
            Data::Queued(p) => Ok(p.rows.clone()),
            Data::Encoded { types, n, bytes } => {
                let width = types.len();
                let mut cols = vec![Vec::new(); width];
                super::cross_rows::decode(width, *n, bytes, &mut cols)?;
                let mut rows = Vec::with_capacity(width * n);
                for r in 0..if width == 0 { 0 } else { *n } {
                    rows.extend(cols.iter().map(|c| c[r]));
                }
                Ok(Rows { types: types.clone(), rows, n: *n })
            }
            Data::Metadata => Err(super::StoreError::Invalid("frame payload was not admitted".into())),
        }
    }

    /// Its rows' count, where its rows were admitted.
    pub fn len(&self) -> Option<usize> {
        match &self.data {
            Data::Encoded { n, .. } => Some(*n),
            Data::Queued(p) => Some(p.rows.n),
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
            Data::Queued(_) | Data::Metadata => 0,
        }
    }
}

// Stay below even SQLite's older 999-parameter limit. Full batches share one prepared shape.
const READ_BATCH: usize = 128;

/// Every frame of a store as this process holds it (decision 0104: a run reads before its first wave): read once
/// (`preload_frames`), kept current by every write this process queues or makes, by space, then relation and part.
/// Shared by the store's handles in this process, by its path.
type Snapshot = HashMap<[u8; 32], HashMap<(Rel, i64), Arc<Admitted>>>;

fn snapshots() -> &'static std::sync::Mutex<HashMap<std::path::PathBuf, Arc<std::sync::Mutex<Snapshot>>>> {
    static SNAPSHOTS: std::sync::OnceLock<std::sync::Mutex<HashMap<std::path::PathBuf, Arc<std::sync::Mutex<Snapshot>>>>> = std::sync::OnceLock::new();
    SNAPSHOTS.get_or_init(Default::default)
}

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

impl ContentStore {
    /// The snapshot of this store's file, by its canonical path: handles that name it differently share it.
    fn snapshot(&self) -> Option<Arc<std::sync::Mutex<Snapshot>>> {
        snapshots().lock().expect("the frames' snapshots").get(&super::atoms::key(&self.database_path)).cloned()
    }

    /// Reads every frame of the store, once a process (decision 0104: before a run's first wave): from then on the
    /// frames are read from memory, and every write of this process updates it. Returns the frames and their
    /// encoded bytes read now (none when an earlier run of the process read them).
    pub fn preload_frames(&self) -> StoreResult<(usize, usize)> {
        if self.snapshot().is_some() {
            return Ok((0, 0));
        }
        let mut snapshot = Snapshot::new();
        let (mut frames, mut bytes) = (0, 0);
        if self.frames_table()? {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare("SELECT space, rel, ident, part, digest, reads, rows, types, data FROM frames")?;
            let mut chains: HashMap<[u8; 32], HashMap<Rel, u32>> = HashMap::new();
            let rows = st.query_map([], |row| {
                let (space, ident): (Vec<u8>, Vec<u8>) = (row.get(0)?, row.get(2)?);
                let space: [u8; 32] = space.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let ident: [u8; 32] = ident.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                let data: Vec<u8> = row.get(8)?;
                let (reads, k) = decode_reads_chain(&row.get::<_, Vec<u8>>(5)?);
                Ok((space, (Rel { name: row.get(1)?, ident }, row.get::<_, i64>(3)?), k, Arc::new(Admitted {
                    stored: Stored { digest: row.get::<_, i64>(4)? as u64, reads },
                    data: Data::Encoded { n: row.get::<_, i64>(6)? as usize, types: row.get(7)?, bytes: Arc::new(data) },
                })))
            })?;
            for row in rows {
                let (space, key, k, frame) = row?;
                frames += 1;
                bytes += frame.encoded_bytes();
                if k > 0 && key.1 == 0 {
                    chains.entry(space).or_default().insert(key.0.clone(), k);
                }
                snapshot.entry(space).or_default().insert(key, frame);
            }
            drop(st);
            drop(connection);
            for (space, held) in snapshot.iter_mut() {
                compose_chains(held, chains.get(space))?;
            }
        }
        snapshots().lock().expect("the frames' snapshots").entry(super::atoms::key(&self.database_path)).or_insert_with(|| Arc::new(std::sync::Mutex::new(snapshot)));
        Ok((frames, bytes))
    }

    fn frames_table(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let n: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='frames'", [], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The stored frames of `rels` in `space`, partition `part`, without their rows.
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
        // Read before the first wave (`preload_frames`): from memory.
        if let Some(snapshot) = self.snapshot() {
            let snapshot = snapshot.lock().expect("the frames' snapshot");
            if let Some(held) = snapshot.get(space) {
                for key in missing {
                    if let Some(frame) = held.get(&key) {
                        out.insert(key, Arc::clone(frame));
                    }
                }
            }
            return Ok(out);
        }
        if !self.frames_table()? { return Ok(out); }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let missing: Vec<_> = missing.into_iter().collect();
        let mut chained: Vec<(Rel, u32)> = Vec::new();
        for batch in missing.chunks(READ_BATCH) {
            let values = (0..batch.len()).map(|i| format!("(?{}, ?{}, ?{})", 2 + 3*i, 3 + 3*i, 4 + 3*i)).collect::<Vec<_>>().join(",");
            let columns = if payload { ", f.rows, f.types, f.data" } else { "" };
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
                Ok(((Rel { name: row.get(0)?, ident }, row.get(2)?), k, Arc::new(Admitted {
                    stored: Stored { digest: row.get::<_, i64>(3)? as u64, reads },
                    data: if payload { Data::Encoded { n: row.get::<_, i64>(5)? as usize, types: row.get(6)?, bytes: Arc::new(row.get(7)?) } } else { Data::Metadata },
                })))
            })?;
            for frame in frames {
                let (key, k, frame) = frame?;
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
                    SELECT f.rel, f.ident, f.part, f.digest, f.reads, f.rows, f.types, f.data FROM demanded d \
                    CROSS JOIN frames f ON f.space = ?1 AND f.rel = d.rel AND f.ident = d.ident AND f.part >= -d.k AND f.part < 0");
                let mut args: Vec<rusqlite::types::Value> = vec![space.to_vec().into()];
                for (rel, k) in batch { args.extend([rel.name.clone().into(), rel.ident.to_vec().into(), (*k as i64).into()]); }
                let mut st = connection.prepare_cached(&sql)?;
                let rows = st.query_map(rusqlite::params_from_iter(args), |row| {
                    let ident: Vec<u8> = row.get(1)?;
                    let ident: [u8; 32] = ident.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok((Rel { name: row.get(0)?, ident }, row.get::<_, i64>(2)?, Arc::new(Admitted {
                        stored: Stored { digest: row.get::<_, i64>(3)? as u64, reads: decode_reads(&row.get::<_, Vec<u8>>(4)?) },
                        data: Data::Encoded { n: row.get::<_, i64>(5)? as usize, types: row.get(6)?, bytes: Arc::new(row.get(7)?) },
                    })))
                })?;
                for row in rows {
                    let (rel, part, frame) = row?;
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

    /// Every stored frame of `rel` in `space`, by part, admitted (queued writes are not among them).
    pub fn admit_all_parts(&self, space: &[u8; 32], rel: &Rel) -> StoreResult<Vec<(i64, Arc<Admitted>)>> {
        if let Some(snapshot) = self.snapshot() {
            let snapshot = snapshot.lock().expect("the frames' snapshot");
            let mut parts: Vec<(i64, Arc<Admitted>)> = snapshot.get(space).into_iter().flatten().filter(|((r, _), _)| r == rel).map(|((_, p), f)| (*p, Arc::clone(f))).collect();
            parts.sort_by_key(|(p, _)| *p);
            return Ok(parts);
        }
        if !self.frames_table()? {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut st = connection.prepare_cached(
            "SELECT part, digest, reads, rows, types, data FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 ORDER BY part",
        )?;
        let rows = st.query_map(params![&space[..], rel.name, &rel.ident[..]], admitted_part)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Run `token`'s use of `parts` of `rel` in `space`, queued behind the run: the space counts the run once, the
    /// parts are marked as used by it, and the parts of `rel` no run used in the last `keep` runs of the space are
    /// forgotten (an incremental store returns its dead rows).
    pub fn queue_frame_parts_used(&self, space: [u8; 32], rel: Rel, parts: Vec<i64>, token: u64, keep: i64) {
        let bytes = parts.len() * 8;
        self.write_back("the frames' parts used", bytes, Vec::new(), Box::new(move |store| store.use_frame_parts(&space, &rel, &parts, token, keep)));
    }

    /// `queue_frame_parts_used`, now.
    pub fn use_frame_parts(&self, space: &[u8; 32], rel: &Rel, parts: &[i64], token: u64, keep: i64) -> StoreResult<()> {
        self.immediate_transaction_once(|t| {
            t.execute_batch(SCHEMA)?;
            let run = count_run(t, space, token)?;
            // Give SQLite the entire key set in one prepared statement. Each run
            // orders it once, without rebuilding SQL and temporary sets per chunk.
            if !parts.is_empty() {
                let parts: rusqlite::vtab::array::Array = std::rc::Rc::new(parts.iter().copied().map(rusqlite::types::Value::from).collect());
                t.prepare_cached("UPDATE frames SET used = ?4 WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part IN rarray(?5)")?
                    .execute(params![&space[..], rel.name, &rel.ident[..], run, parts])?;
            }
            t.execute("DELETE FROM frames WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND used <= ?4", params![&space[..], rel.name, &rel.ident[..], run - keep])?;
            Ok(())
        })
    }

    /// Forgets every frame of `space` and its count of runs: the space of a program no run will run again.
    pub fn drop_frame_space(&self, space: &[u8; 32]) -> StoreResult<usize> {
        if let Some(snapshot) = self.snapshot() {
            snapshot.lock().expect("the frames' snapshot").remove(space);
        }
        if !self.frames_table()? {
            return Ok(0);
        }
        self.immediate_transaction_once(|t| {
            let n = t.execute("DELETE FROM frames WHERE space = ?1", [&space[..]])?;
            t.execute("DELETE FROM frame_spaces WHERE space = ?1", [&space[..]])?;
            Ok(n)
        })
    }

    /// `keep_frames`, queued behind the run (decision 0102): encoded and written by the store's writer.
    pub fn queue_frames(&self, space: [u8; 32], puts: Vec<Put>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        self.queue_frames_retagged(space, puts, Vec::new(), Vec::new(), gone, used)
    }

    /// `queue_frames`, with `retags` (frames whose rows stay, with their reads told again: no rows are written) and
    /// `deltas` (a relation's rows gone and added, written after its frame: decision 0117 step 4). A frame put whole
    /// (part 0) ends the deltas after it.
    pub fn queue_frames_retagged(&self, space: [u8; 32], puts: Vec<Put>, retags: Vec<Retag>, deltas: Vec<Delta>, gone: Vec<(Rel, i64)>, used: Option<Used>) {
        // Each delta after its relation's frame as it is now (in memory, queued or stored).
        let mut delta_puts: Vec<Arc<Put>> = Vec::new();
        let mut heads: Vec<(Rel, u64, Vec<(String, u64)>)> = Vec::new();
        // A relation's deltas queued in this call so far (two of one relation follow each other).
        let mut composed: Vec<(Rel, Arc<Admitted>)> = Vec::new();
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
            let put = Arc::new(Put { rel: d.rel.clone(), part: delta_part(k + 1), digest: 0, reads: Vec::new(), rows: Rows { types, rows, n: ng + na } });
            let frame = Arc::new(Admitted { stored: Stored { digest: 0, reads: Vec::new() }, data: Data::Queued(Arc::clone(&put)) });
            composed.push((d.rel.clone(), Arc::new(now.with_delta(Stored { digest: d.digest, reads: d.reads.clone() }, frame, ng, na))));
            let mut stored_reads = d.reads.clone();
            stored_reads.push((CHAIN.to_owned(), (k + 1) as u64));
            heads.push((d.rel, d.digest, stored_reads));
            delta_puts.push(put);
        }
        let bytes: usize = puts.iter().map(|p| p.rows.rows.len() * 8).sum::<usize>() + delta_puts.iter().map(|p| p.rows.rows.len() * 8).sum::<usize>();
        // What a large write of frames holds, by relation (profiling only): what makes the store's writes large.
        if bytes > 16 << 20 && std::env::var_os("RERAIL_PROFILE").is_some() {
            let mut by: HashMap<&str, (usize, usize, usize)> = HashMap::new();
            for p in &puts {
                let e = by.entry(p.rel.name.as_str()).or_default();
                e.0 += 1;
                e.1 += p.rows.n;
                e.2 += p.rows.rows.len() * 8;
            }
            let mut by: Vec<_> = by.into_iter().collect();
            by.sort_by(|a, b| b.1.2.cmp(&a.1.2));
            let top: Vec<String> = by.iter().take(15).map(|(r, (f, n, b))| format!("{r}: {f} frames, {n} rows, {:.1} MB", *b as f64 / 1048576.0)).collect();
            eprintln!("rerail: frames queued: {:.1} MB in {} frames of {} relations; by relation: {}", bytes as f64 / 1048576.0, puts.len(), by.len(), top.join("; "));
            if let Some(dir) = std::env::var_os("RERAIL_PROFILE") {
                let mut tsv = String::from("relation\tframes\trows\tbytes\n");
                for (r, (f, n, b)) in &by {
                    tsv.push_str(&format!("{r}\t{f}\t{n}\t{b}\n"));
                }
                let _ = std::fs::write(std::path::Path::new(&dir).join(format!("frames-{}.tsv", bytes)), tsv);
            }
        }
        let mut gone = gone;
        gone.extend(self.ended_deltas(&space, puts.iter()));
        let puts: Vec<_> = puts.into_iter().map(Arc::new).collect();
        let mut keys: Vec<(super::write_back::Key, Option<Arc<dyn std::any::Any + Send + Sync>>)> = puts.iter().map(|p| {
            let frame = Arc::new(Admitted { stored: Stored { digest: p.digest, reads: p.reads.clone() }, data: Data::Queued(Arc::clone(p)) });
            (frame_key(&space, &p.rel, p.part), Some(Arc::new(Some(frame)) as Arc<dyn std::any::Any + Send + Sync>))
        }).collect();
        keys.extend(gone.iter().map(|(rel, part)| (frame_key(&space, rel, *part), Some(Arc::new(None::<Arc<Admitted>>) as Arc<dyn std::any::Any + Send + Sync>))));
        keys.extend(composed.iter().map(|(rel, f)| (frame_key(&space, rel, 0), Some(Arc::new(Some(Arc::clone(f))) as Arc<dyn std::any::Any + Send + Sync>))));
        // Memory is ahead of the store: the snapshot holds what is queued (`preload_frames`).
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
            for p in &puts {
                held.insert((p.rel.clone(), p.part), Arc::new(Admitted { stored: Stored { digest: p.digest, reads: p.reads.clone() }, data: Data::Queued(Arc::clone(p)) }));
            }
            for key in &gone {
                held.remove(key);
            }
        }
        self.write_back("frames", bytes, keys, Box::new(move |store| {
            // Attempted once: a busy store leaves these frames unwritten, and the next run derives them again.
            let refs: Vec<_> = puts.iter().map(Arc::as_ref).collect();
            let delta_refs: Vec<_> = delta_puts.iter().map(Arc::as_ref).collect();
            let _ = store.keep_frame_refs(&space, &refs, &retags, &delta_refs, &heads, &gone, used.as_ref().map(|(rels, run)| (rels.as_slice(), *run)));
            Ok(())
        }));
    }

    /// Writes `puts` into `space` (each replacing its frame) and forgets `gone`. With `used`, a program of run `token`
    /// is over: the space counts the run if it has not yet, marks the relations the program used, and forgets those
    /// the last [`RUNS`] runs did not use.
    pub fn keep_frames(&self, space: &[u8; 32], puts: &[Put], gone: &[(Rel, i64)], used: Option<(&[Rel], u64)>) -> StoreResult<()> {
        let mut gone = gone.to_vec();
        gone.extend(self.ended_deltas(space, puts.iter()));
        self.keep_frame_refs(space, &puts.iter().collect::<Vec<_>>(), &[], &[], &[], &gone, used)
    }

    /// The delta frames that `puts` end (decision 0117 step 4): those after a frame put whole, as the snapshot holds
    /// them. Without a snapshot none is named; its reads name no deltas, so they are not read, and the next delta
    /// writes over them.
    fn ended_deltas<'p>(&self, space: &[u8; 32], puts: impl Iterator<Item = &'p Put>) -> Vec<(Rel, i64)> {
        let Some(snapshot) = self.snapshot() else { return Vec::new() };
        let snapshot = snapshot.lock().expect("the frames' snapshot");
        let Some(held) = snapshot.get(space) else { return Vec::new() };
        let mut out = Vec::new();
        for p in puts.filter(|p| p.part == 0) {
            if let Some(f) = held.get(&(p.rel.clone(), 0)) {
                out.extend((1..=f.chain().0).map(|i| (p.rel.clone(), delta_part(i))));
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn keep_frame_refs(&self, space: &[u8; 32], puts: &[&Put], retags: &[Retag], delta_puts: &[&Put], heads: &[(Rel, u64, Vec<(String, u64)>)], gone: &[(Rel, i64)], used: Option<(&[Rel], u64)>) -> StoreResult<()> {
        let blocks: Vec<Vec<u8>> = puts.iter().map(|p| super::cross_rows::encode(p.rows.types.len(), &p.rows.rows)).collect();
        let delta_blocks: Vec<Vec<u8>> = delta_puts.iter().map(|p| super::cross_rows::encode(p.rows.types.len(), &p.rows.rows)).collect();
        // The snapshot holds what is written (a write the writer runs was put there as it was queued: the same).
        if let Some(snapshot) = self.snapshot() {
            let mut snapshot = snapshot.lock().expect("the frames' snapshot");
            let held = snapshot.entry(*space).or_default();
            for (p, block) in puts.iter().zip(&blocks) {
                held.insert((p.rel.clone(), p.part), Arc::new(Admitted { stored: Stored { digest: p.digest, reads: p.reads.clone() }, data: Data::Encoded { types: p.rows.types.clone(), n: p.rows.n, bytes: Arc::new(block.clone()) } }));
            }
            for key in gone {
                held.remove(key);
            }
        }
        self.immediate_transaction_once(|t| {
            t.execute_batch(SCHEMA)?;
            // A run's first program counts it: what it writes and marks is of the new run.
            let run = match used {
                Some((_, token)) => count_run(t, space, token)?,
                None => {
                    t.execute("INSERT INTO frame_spaces(space, runs) VALUES (?1, 0) ON CONFLICT(space) DO NOTHING", [&space[..]])?;
                    t.query_row("SELECT runs FROM frame_spaces WHERE space = ?1", [&space[..]], |r| r.get(0))?
                }
            };
            // A frame put whole has no deltas after it: its reads name none (`CHAIN`), so those left are not read.
            // Nine bound values per frame, plus shared space and run, stay below 999 parameters.
            // Each statement publishes a bounded batch. No frame launches its own SQL execution.
            for batch in puts.iter().zip(&blocks).chain(delta_puts.iter().zip(&delta_blocks)).collect::<Vec<_>>().chunks(96) {
                let values = (0..batch.len()).map(|i| {
                    let p=3+9*i;
                    format!("(?1,?{p},?{},?{},?{},?{},?{},?{},?{},?{},?2)",p+1,p+2,p+3,p+4,p+5,p+6,p+7,p+8)
                }).collect::<Vec<_>>().join(",");
                let sql=format!("INSERT INTO frames(space,rel,ident,part,digest,reads,width,rows,types,data,used) VALUES {values} \
                    ON CONFLICT(space,rel,ident,part) DO UPDATE SET digest=excluded.digest, reads=excluded.reads, \
                    width=excluded.width, rows=excluded.rows, types=excluded.types, data=excluded.data, used=excluded.used");
                let mut args: Vec<rusqlite::types::Value>=vec![space.to_vec().into(),run.into()];
                for (p,data) in batch {
                    args.extend([p.rel.name.clone().into(),p.rel.ident.to_vec().into(),p.part.into(),(p.digest as i64).into(),
                        encode_reads(&p.reads).into(),(p.rows.types.len() as i64).into(),(p.rows.n as i64).into(),p.rows.types.clone().into(),(*data).clone().into()]);
                }
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
            }
            // A relation with a delta after its frame: the frame's digest and reads are the relation's now.
            for (rel, digest, reads) in heads {
                t.prepare_cached("UPDATE frames SET digest = ?5, reads = ?6, used = ?7 WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?
                    .execute(rusqlite::params![&space[..], rel.name, &rel.ident[..], 0i64, *digest as i64, encode_reads(reads), run])?;
            }
            // A retagged frame: its reads only; its rows, digest and the rest stay as stored, with the count of its
            // deltas (`CHAIN`, the stored reads' last entry: 4 + 7 + 8 bytes).
            for r in retags {
                t.prepare_cached("UPDATE frames SET reads = CASE WHEN substr(reads, -19, 11) = ?7 THEN CAST(?5 || substr(reads, -19) AS BLOB) ELSE ?5 END, used = ?6 \
                    WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")?
                    .execute(rusqlite::params![&space[..], r.rel.name, &r.rel.ident[..], r.part, encode_reads(&r.reads), run, chain_mark()])?;
            }
            for batch in gone.chunks(READ_BATCH) {
                let values=(0..batch.len()).map(|i|format!("(?{},?{},?{})",2+3*i,3+3*i,4+3*i)).collect::<Vec<_>>().join(",");
                let sql=format!("WITH demanded(rel,ident,part) AS (VALUES {values}) \
                    DELETE FROM frames WHERE (space,rel,ident,part) IN (SELECT ?1,rel,ident,part FROM demanded)");
                let mut args: Vec<rusqlite::types::Value>=vec![space.to_vec().into()];
                for (rel,part) in batch { args.extend([rel.name.clone().into(),rel.ident.to_vec().into(),(*part).into()]); }
                t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
            }
            if let Some((used, _)) = used {
                for batch in used.chunks(READ_BATCH) {
                    let values=(0..batch.len()).map(|i|format!("(?{},?{})",3+2*i,4+2*i)).collect::<Vec<_>>().join(",");
                    // Row-value membership selects primary-key prefixes, including every partition of a relation.
                    let sql=format!("WITH demanded(rel,ident) AS (VALUES {values}) \
                        UPDATE frames SET used=?2 WHERE (space,rel,ident) IN (SELECT ?1,rel,ident FROM demanded)");
                    let mut args: Vec<rusqlite::types::Value>=vec![space.to_vec().into(),run.into()];
                    for rel in batch { args.extend([rel.name.clone().into(),rel.ident.to_vec().into()]); }
                    t.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(args))?;
                }
                t.execute("DELETE FROM frames WHERE space = ?1 AND used <= ?2", params![&space[..], run - RUNS])?;
            }
            Ok(())
        })
    }
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

/// The space of the per-file memo of plan `plan` (decision 0108): one space per file-local program.
pub fn local_space(plan: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.space/v1\0local\0");
    h.update(plan);
    h.finalize().into()
}

/// The run of `space` that run `token` is: counted once per token (`0`: a run of its own, counted each time).
fn count_run(t: &rusqlite::Connection, space: &[u8; 32], token: u64) -> rusqlite::Result<i64> {
    t.execute("INSERT INTO frame_spaces(space, runs) VALUES (?1, 0) ON CONFLICT(space) DO NOTHING", [&space[..]])?;
    let run: i64 = t.query_row("SELECT runs FROM frame_spaces WHERE space = ?1", [&space[..]], |r| r.get(0))?;
    let mut counted = counted().lock().expect("the spaces' counted runs");
    if token == 0 || counted.get(space) != Some(&token) {
        t.execute("UPDATE frame_spaces SET runs = runs + 1 WHERE space = ?1", [&space[..]])?;
        counted.insert(*space, token);
        return Ok(run + 1);
    }
    Ok(run)
}

/// A frame read by part: `(part, digest, reads, rows, types, data)`.
fn admitted_part(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, Arc<Admitted>)> {
    Ok((row.get(0)?, Arc::new(Admitted {
        stored: Stored { digest: row.get::<_, i64>(1)? as u64, reads: decode_reads(&row.get::<_, Vec<u8>>(2)?) },
        data: Data::Encoded { n: row.get::<_, i64>(3)? as usize, types: row.get(4)?, bytes: Arc::new(row.get(5)?) },
    })))
}

/// The entry a base frame's stored reads end with when deltas follow it (decision 0117 step 4): how many. A frame
/// put whole has none, so deltas a put left behind are not read.
const CHAIN: &str = "~deltas";

/// How `CHAIN`'s entry starts in stored reads: its name's length and its name.
fn chain_mark() -> Vec<u8> {
    let mut out = (CHAIN.len() as u32).to_le_bytes().to_vec();
    out.extend(CHAIN.as_bytes());
    out
}

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
    fn frame_part_usage_is_one_bulk_update_and_preserves_retention() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let relation = rel("local", 1);
        let puts: Vec<_> = (0..8193).map(|part| Put { rel: relation.clone(), part,
            digest: 7, reads: vec![("input-source-identity".into(),7);8], rows: Rows { types: vec![0], rows: vec![part], n: 1 } }).collect();
        store.keep_frames(&[42;32], &puts, &[], None).unwrap();
        // These share either the space or the relation, but are outside the requested identity.
        store.keep_frames(&[43;32], &puts[..1], &[], None).unwrap();
        let other = Put { rel: rel("local",2), part:8192, digest:7, reads:vec![], rows:puts[0].rows.clone() };
        store.keep_frames(&[42;32], &[other], &[], None).unwrap();
        let pages = store.connection.lock().unwrap().query_row("PRAGMA page_count",[],|r|r.get::<_,i64>(0)).unwrap() as usize;
        let cache_writes = || {
            let (mut now,mut peak)=(0,0);
            unsafe { rusqlite::ffi::sqlite3_db_status(store.connection.lock().unwrap().handle(),
                rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,&mut now,&mut peak,1); }
            now as usize
        };
        cache_writes();
        let counts = [AtomicUsize::new(0), AtomicUsize::new(0)];
        unsafe extern "C" fn trace(event:u32, state:*mut std::ffi::c_void, stmt:*mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
            let c = unsafe { &*state.cast::<[AtomicUsize;2]>() };
            if event == rusqlite::ffi::SQLITE_TRACE_STMT {
                c[0].fetch_add(1,Ordering::Relaxed);
                unsafe { rusqlite::ffi::sqlite3_stmt_status(stmt.cast(),rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,1) };
            } else {
                let vm = unsafe { rusqlite::ffi::sqlite3_stmt_status(stmt.cast(),rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,1) };
                c[1].fetch_add(vm as usize,Ordering::Relaxed);
            }
            0
        }
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),
            rusqlite::ffi::SQLITE_TRACE_STMT | rusqlite::ffi::SQLITE_TRACE_PROFILE, Some(trace),
            (&counts as *const [AtomicUsize;2]).cast_mut().cast()); }
        // File order and digest-derived partition order are unrelated.
        let parts: Vec<_> = (0..8192).map(|i| i*4051%8192).chain([7,7,-1]).collect();
        let result=store.use_frame_parts(&[42;32],&relation,&parts,0,2);
        unsafe { rusqlite::ffi::sqlite3_trace_v2(store.connection.lock().unwrap().handle(),0,None,std::ptr::null_mut()); }
        result.unwrap();
        let statements=counts[0].load(Ordering::Relaxed);
        let vm=counts[1].load(Ordering::Relaxed);
        let writes=cache_writes();
        eprintln!("frame parts used: {statements} statements, {vm} VM instructions, {writes} page writes for {pages} database pages");
        assert!(statements<=10,"usage must be one bulk update, independent of key count: {statements}");
        assert!(vm<500_000,"usage must stay linear in keys: {vm}");
        assert!(writes<2*pages,"bulk usage must not spill the same pages across key chunks: {writes} writes for {pages} pages");
        assert!(store.frame_rows(&[42;32],&relation,8192).unwrap().is_some());
        store.use_frame_parts(&[42;32],&relation,&parts,0,2).unwrap();
        assert!(store.frame_rows(&[42;32],&relation,8192).unwrap().is_none(),"an unused part expires after two runs");
        assert!(store.frame_rows(&[42;32],&relation,8191).unwrap().is_some());
        assert!(store.frame_rows(&[43;32],&relation,0).unwrap().is_some());
        assert!(store.frame_rows(&[42;32],&rel("local",2),8192).unwrap().is_some());
        store.use_frame_parts(&[42;32],&relation,&[],0,2).unwrap();
        assert!(store.frame_rows(&[42;32],&relation,8191).unwrap().is_some());
        store.use_frame_parts(&[42;32],&relation,&[],0,2).unwrap();
        assert!(store.frame_rows(&[42;32],&relation,8191).unwrap().is_none());
    }

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
        let refs: Vec<_> = changed.iter().collect();
        let result = store.keep_frame_refs(&[1;32], &refs, &[], &[], &[], &gone, Some((&used, 1)));
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
        // A frame retagged keeps its deltas.
        store.queue_frames_retagged([1; 32], vec![], vec![Retag { rel: r.clone(), part: 0, reads: vec![("b".into(), 4)] }], vec![], vec![], None);
        drop(store);
        let store = ContentStore::open(&path).unwrap();
        let got = store.admit_frames(&[1; 32], &[r.clone()], 0).unwrap();
        assert_eq!((got[&r].stored.reads.clone(), got[&r].chain()), (vec![("b".into(), 4)], (2, 4)));
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

    #[test]
    fn queued_frames_are_read_without_waiting_for_the_writer() {
        use std::sync::mpsc;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let store = ContentStore::open(&path).unwrap();
        let reader = ContentStore::open(&path).unwrap();
        let (release, blocked) = mpsc::channel();
        store.write_back("test gate", 0, vec![], Box::new(move |_| { blocked.recv().unwrap(); Ok(()) }));
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
        store.write_back("test gate", 0, vec![], Box::new(move |_| { blocked.recv().unwrap(); Ok(()) }));
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

    fn rel(name: &str, i: u8) -> Rel {
        Rel { name: name.into(), ident: [i; 32] }
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

    #[test]
    fn a_relation_no_run_used_for_four_runs_is_collected() {
        let dir = std::env::temp_dir().join(format!("rerail-frames-gc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let space = [2u8; 32];
        let (kept, old) = (rel("kept", 1), rel("old", 1));
        let put = |r: &Rel| Put { rel: r.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        store.keep_frames(&space, &[put(&kept), put(&old)], &[], Some((&[kept.clone(), old.clone()][..], 0))).unwrap();
        for _ in 0..RUNS {
            assert!(store.frames(&space, &[old.clone()], 0).unwrap().contains_key(&old));
            store.keep_frames(&space, &[], &[], Some((&[kept.clone()][..], 0))).unwrap();
        }
        let got = store.frames(&space, &[kept.clone(), old.clone()], 0).unwrap();
        assert!(got.contains_key(&kept) && !got.contains_key(&old));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The programs of one run that keep frames in one space (a phase's, P3's cross part, its loading cone) count one
    /// run: however many there are, a relation one of them used is kept for the next [`RUNS`] runs.
    #[test]
    fn the_programs_of_one_run_count_one_run_of_their_space() {
        let dir = std::env::temp_dir().join(format!("rerail-frames-runs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let space = [3u8; 32];
        let (first, other) = (rel("first", 1), rel("other", 1));
        let put = |r: &Rel| Put { rel: r.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } };
        // Run 7's first program keeps `first`; eight more programs of run 7 keep `other`.
        store.keep_frames(&space, &[put(&first)], &[], Some((&[first.clone()][..], 7))).unwrap();
        for _ in 0..2 * RUNS {
            store.keep_frames(&space, &[put(&other)], &[], Some((&[other.clone()][..], 7))).unwrap();
        }
        assert!(store.frames(&space, &[first.clone()], 0).unwrap().contains_key(&first), "one run, not nine");
        // RUNS later runs that do not use it collect it.
        for run in 8..8 + RUNS as u64 {
            store.keep_frames(&space, &[], &[], Some((&[other.clone()][..], run))).unwrap();
        }
        assert!(!store.frames(&space, &[first.clone()], 0).unwrap().contains_key(&first));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A store created at `path` (its schema new): a snapshot of an earlier file there is not this one's.
pub fn forget_snapshot(path: &str) {
    snapshots().lock().expect("the frames' snapshots").remove(&super::atoms::key(std::path::Path::new(path)));
}
