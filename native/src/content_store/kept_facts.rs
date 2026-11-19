//! A space's kept facts as blocks (decision 0401 F1–F6): a base and one cumulative delta in `<store>.facts/<space>/`,
//! each a `rerail_blocks::kept` block, mapped by every process that reads the space. SQLite keeps which two files a
//! space has (`facts_blocks`), replaced in the transaction that commits a run's writes.
//!
//! The store's writer writes a space's next block behind the run (`write_space`): the previous delta's entries not
//! replaced are carried, the run's are put, and a base the delta has outgrown is rolled into a new one. Each file is
//! made under a name no reader opens, frozen, renamed into place and its directory synced before its row names it.
//! The files a committed write replaced are removed by the next write (`sweep`): the transaction may still roll back
//! when this one returns, and a process that mapped one keeps reading it.

use super::atoms::stored::{clone_file, new_tag, private_dir, sync_dir};
use super::{StoreError, StoreResult};
use rerail_blocks::kept::{FactKey, KeptFacts, KeptRead, KeptView, KeptW, Meta};
use rerail_blocks::relation::Relation;
use rusqlite::{Connection, OptionalExtension as _};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A relation the keep puts, lent by its holder (the engine's run): its rows, dead bits and levels, kept alive while
/// this lives. Nothing writes them meanwhile.
pub trait KeptRows: Send + Sync {
    fn relation(&self) -> &Relation;
    fn dead(&self) -> &[u64];
    fn levels(&self) -> &[u32];
}

/// What a put is.
pub enum KeptWhat {
    /// A relation the run derived, copied whole.
    Rows(Box<dyn KeptRows>),
    /// Rows not held as a relation (an input's granule): `n` rows of `widths.len()` columns, flat.
    Flat { widths: Vec<u32>, rows: Vec<i64>, n: usize },
    /// The kept entry's rows as they are, with these reads (a frame's retag).
    Reads,
    /// No longer kept.
    Gone,
}

/// One put of a space's next block.
pub struct KeptPut {
    pub rel: String,
    pub ident: [u8; 32],
    pub part: i64,
    pub digest: u64,
    pub reads: Vec<(String, u64)>,
    /// Each column's value type (the engine's codes: `s` an atom).
    pub types: Vec<u8>,
    pub what: KeptWhat,
}

impl KeptPut {
    fn key(&self) -> FactKey<'_> {
        FactKey { rel: &self.rel, ident: &self.ident, part: self.part }
    }
}

/// The directory of a store's kept facts.
pub fn facts_dir(store: &Path) -> PathBuf {
    let mut p = store.as_os_str().to_owned();
    p.push(".facts");
    PathBuf::from(p)
}

fn space_dir(store: &Path, space: &[u8; 32]) -> PathBuf {
    facts_dir(store).join(space.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// A space's two files as its row names them: (name, tag).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpaceRow {
    pub base: Option<(String, u64)>,
    pub delta: Option<(String, u64)>,
}

/// The row of `space`, if it keeps blocks.
pub fn space_row(connection: &Connection, space: &[u8; 32]) -> StoreResult<Option<SpaceRow>> {
    Ok(connection
        .prepare_cached("SELECT base, base_tag, delta, delta_tag FROM main.facts_blocks WHERE space = ?1")?
        .query_row([&space[..]], |r| {
            let file = |name: Option<String>, tag: Option<i64>| name.zip(tag).map(|(n, t)| (n, t as u64));
            Ok(SpaceRow { base: file(r.get(0)?, r.get(1)?), delta: file(r.get(2)?, r.get(3)?) })
        })
        .optional()?)
}

/// A space's blocks, mapped.
#[derive(Default)]
pub struct KeptSpace {
    /// Shared with the readers that bind its relations where they lie (`Block::share`).
    pub base: Option<std::sync::Arc<KeptFacts>>,
    pub delta: Option<std::sync::Arc<KeptFacts>>,
}

impl KeptSpace {
    /// The blocks `row` names, mapped, each checked against its tag.
    pub fn open(store: &Path, space: &[u8; 32], row: &SpaceRow) -> StoreResult<KeptSpace> {
        let dir = space_dir(store, space);
        let open = |f: &Option<(String, u64)>| -> StoreResult<Option<std::sync::Arc<KeptFacts>>> {
            f.as_ref()
                .map(|(name, tag)| KeptFacts::open(&dir.join(name), *tag).map(std::sync::Arc::new).map_err(|e| StoreError::Corrupt(format!("a space's kept facts: {e}"))))
                .transpose()
        };
        let space = KeptSpace { base: open(&row.base)?, delta: open(&row.delta)? };
        if let (Some(b), Some(d)) = (&space.base, &space.delta)
            && d.base_tag() != Some(b.tag())
        {
            return Err(StoreError::Corrupt("a space's kept delta is not of its base".into()));
        }
        Ok(space)
    }

    /// The space as one.
    pub fn view(&self) -> KeptView<'_> {
        KeptView { base: self.base.as_deref(), delta: self.delta.as_deref() }
    }
}

/// Removes the files of `dir` that `row` (read in the write transaction) does not name: the blocks a committed write
/// replaced, a rolled-back write's, a crashed maker's under its making name. A process mapping one keeps reading it.
fn sweep(dir: &Path, row: &SpaceRow) {
    let named: HashSet<&str> = row.base.iter().chain(&row.delta).map(|(n, _)| n.as_str()).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !named.contains(name.as_str()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A block's room: what its relations hold, with each chunk's alignment, and room for its directory. Its file is
/// sparse and truncated at its freeze, so room not written costs no disk.
fn room_for(rel: &Relation) -> usize {
    let chunks = rel.rows.nchunks.load(std::sync::atomic::Ordering::Relaxed) as usize + rel.killed.nchunks.load(std::sync::atomic::Ordering::Relaxed) as usize;
    // A continuation's copy holds its chain's rows too (decision 0384 step 7): their values, and chunks enough for every
    // row it reads (four a doubling group).
    let chain = if rel.continues() { 4 * (u32::BITS - rel.len_all().leading_zeros()) as usize } else { 0 };
    rel.held_bytes() + rel.chain_value_bytes() + Relation::descriptor_bytes(rel.arity as usize) + ((chunks + chain) * (rel.arity as usize + 1) + rel.indexes.len() * 2 + 4) * rerail_blocks::block::PAGE
}

/// Maps a copy's atom columns (`types` `s`) through `map`, where they lie (a forked process's ids to the store's).
fn map_copy(copy: &Relation, types: &[u8], map: &dyn Fn(&mut [i64]) -> StoreResult<()>) -> StoreResult<()> {
    let n = copy.rows.len();
    for (c, _) in types.iter().enumerate().filter(|(_, t)| **t == b's') {
        let mut col = vec![0i64; n as usize];
        copy.rows.read_col(c, n, &mut col);
        map(&mut col)?;
        copy.rows.write_col(c, &col);
    }
    Ok(())
}

/// Writes `space`'s next block in transaction `t`, with `puts` (each key once): a delta over its base, or a new base
/// when there is none or the delta would outgrow it. `map` maps a forked process's atom ids to the store's.
pub fn write_space(store: &Path, t: &Connection, space: &[u8; 32], puts: Vec<KeptPut>, map: Option<&dyn Fn(&mut [i64]) -> StoreResult<()>>) -> StoreResult<()> {
    if puts.is_empty() {
        return Ok(());
    }
    let io = |e: std::io::Error| StoreError::Io(e);
    let dir = space_dir(store, space);
    private_dir(&dir).map_err(io)?;
    let row = space_row(t, space)?.unwrap_or_default();
    sweep(&dir, &row);
    let old = KeptSpace::open(store, space, &row)?;
    let view = old.view();
    let replaced: HashSet<FactKey<'_>> = puts.iter().map(KeptPut::key).collect();
    // Each put's bytes (what a delta weighs against its base), and the room its copy takes in the block.
    let (put_bytes, put_room) = puts
        .iter()
        .map(|p| match &p.what {
            KeptWhat::Rows(r) => {
                let side = r.dead().len() * 8 + r.levels().len() * 4;
                (r.relation().bytes() + side, room_for(r.relation()) + side)
            }
            KeptWhat::Flat { widths, n, .. } => (n * widths.len() * 8, n * widths.len() * 8 + (widths.len() + 1) * 40 * rerail_blocks::block::PAGE),
            KeptWhat::Reads => view.get(&p.key()).map_or((0, 0), |k| (k.rows.bytes(), room_for(k.rows))),
            KeptWhat::Gone => (0, 0),
        })
        .fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
    let carried: Vec<rerail_blocks::kept::Kept<'_>> = match (&old.base, &old.delta) {
        (Some(_), Some(d)) => d.all().filter(|k| !replaced.contains(&k.key)).collect(),
        _ => Vec::new(),
    };
    let carried_bytes: usize = carried.iter().map(|k| k.rows.bytes() + k.dead.len() * 8 + k.levels.len() * 4).sum();
    // A new base when there is none, or when the delta would hold more than its base: every entry the space keeps.
    let roll = old.base.as_ref().is_none_or(|b| put_bytes + carried_bytes > b.bytes());
    let entries: Vec<rerail_blocks::kept::Kept<'_>> = if roll { view.all().filter(|k| !replaced.contains(&k.key)).collect() } else { carried };
    let reserve = (entries.iter().map(|k| room_for(k.rows) + k.dead.len() * 8 + k.levels.len() * 4).sum::<usize>() + put_room) * 5 / 4 + (64 << 20);
    let tag = loop {
        let t = new_tag();
        if t != 0 {
            break t;
        }
    };
    let name = format!("{}{tag:016x}.blk", if roll { 'b' } else { 'd' });
    let making = dir.join(format!("{name}.new"));
    let base_tag = if roll { None } else { old.base.as_deref().map(KeptFacts::tag) };
    let mut w = KeptW::create(&making, tag, base_tag, reserve).map_err(io)?;
    for k in &entries {
        w.carry(k, None).map_err(io)?;
    }
    // A delta carries what its predecessor recorded as gone, unless put again now.
    if !roll && let Some(d) = &old.delta {
        for key in d.gone_keys().filter(|k| !replaced.contains(k)) {
            w.gone(key);
        }
    }
    for p in &puts {
        let reads: Vec<(&str, u64)> = p.reads.iter().map(|(r, d)| (r.as_str(), *d)).collect();
        let meta = |dead, levels| Meta { key: p.key(), digest: p.digest, reads: &reads, types: &p.types, dead, levels };
        match &p.what {
            KeptWhat::Rows(r) => {
                let copy = w.put(meta(r.dead(), r.levels()), r.relation(), None).map_err(io)?;
                if let Some(map) = map {
                    map_copy(copy, &p.types, map)?;
                }
            }
            KeptWhat::Flat { widths, rows, n } => {
                let mut rows = rows.clone();
                if let Some(map) = map
                    && !widths.is_empty()
                {
                    for (c, _) in p.types.iter().enumerate().filter(|(_, t)| **t == b's') {
                        let mut col: Vec<i64> = rows.chunks_exact(widths.len()).map(|r| r[c]).collect();
                        map(&mut col)?;
                        for (r, v) in rows.chunks_exact_mut(widths.len()).zip(col) {
                            r[c] = v;
                        }
                    }
                }
                w.put_rows(meta(&[], &[]), widths, &rows, *n).map_err(io)?;
            }
            KeptWhat::Reads => {
                if let Some(k) = view.get(&p.key()) {
                    w.carry(&k, Some(&reads)).map_err(io)?;
                }
            }
            KeptWhat::Gone => {
                if !roll && old.base.as_ref().is_some_and(|b| b.get(&p.key()).is_some()) {
                    w.gone(p.key());
                }
            }
        }
    }
    w.finish().map_err(io)?;
    std::fs::rename(&making, dir.join(&name)).map_err(io)?;
    sync_dir(&dir).map_err(io)?;
    let (base, delta) = if roll { (Some((name, tag)), None) } else { (row.base.clone(), Some((name, tag))) };
    t.prepare_cached(
        "INSERT INTO main.facts_blocks(space, base, base_tag, delta, delta_tag) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(space) DO UPDATE SET base = excluded.base, base_tag = excluded.base_tag, delta = excluded.delta, delta_tag = excluded.delta_tag",
    )?
    .execute(rusqlite::params![
        &space[..],
        base.as_ref().map(|b| b.0.as_str()),
        base.as_ref().map(|b| b.1 as i64),
        delta.as_ref().map(|d| d.0.as_str()),
        delta.as_ref().map(|d| d.1 as i64),
    ])?;
    Ok(())
}

/// Spaces' next blocks, queued for the store's writer (`ContentStore::queue_kept`): each space's puts of a wave
/// together, written as one block in the wave's transaction.
pub(super) struct KeptWrite {
    spaces: Vec<([u8; 32], Vec<KeptPut>)>,
}

impl KeptWrite {
    pub(super) fn run(&mut self, store: &super::ContentStore) -> StoreResult<()> {
        let spaces = std::mem::take(&mut self.spaces);
        let path = store.database_path.clone();
        store.immediate_transaction_once(|t| {
            // Forked (decision 0170): the copies' atom columns are mapped to the store's ids in this transaction.
            let map = |v: &mut [i64]| store.atoms.to_store(t, super::ProcessIds::of_process(v)).map(drop);
            for (space, puts) in spaces {
                write_space(&path, t, &space, puts, store.atoms.forked().then_some(&map as &dyn Fn(&mut [i64]) -> StoreResult<()>))?;
            }
            Ok(())
        })
    }
}

/// A run's wave's kept puts, by space (`ContentStore::gather`): one block a space a wave, where a block a put made
/// each write copy the space's whole delta again, and sync its file and directory (a cold data-layer run over 50
/// files: 2,442 blocks, 15.7 s of the writer's CPU and most of the run's wall, its queue full at 300 files).
#[derive(Default)]
struct KeptGather {
    spaces: std::collections::BTreeMap<[u8; 32], (Vec<KeptPut>, std::collections::HashMap<(String, [u8; 32], i64), usize>)>,
    bytes: usize,
}

/// The bytes a put holds until written: a flat put's rows, and a little for the rest (a relation is lent, not held).
fn held_bytes(p: &KeptPut) -> usize {
    64 + match &p.what {
        KeptWhat::Flat { rows, .. } => rows.len() * 8,
        _ => 0,
    }
}

impl KeptGather {
    /// `space`'s puts, each key once: a later put replaces an earlier one of its key, but a retag (`Reads`) keeps the
    /// rows an earlier put of this wave gave it, with its reads.
    fn add(&mut self, space: [u8; 32], puts: Vec<KeptPut>) {
        let (list, at) = self.spaces.entry(space).or_default();
        for p in puts {
            self.bytes += held_bytes(&p);
            let key = (p.rel.clone(), p.ident, p.part);
            match at.get(&key) {
                Some(&i) => {
                    let was = &mut list[i];
                    if matches!(p.what, KeptWhat::Reads) && !matches!(was.what, KeptWhat::Reads | KeptWhat::Gone) {
                        was.reads = p.reads;
                        was.digest = p.digest;
                    } else {
                        *was = p;
                    }
                }
                None => {
                    at.insert(key, list.len());
                    list.push(p);
                }
            }
        }
    }
}

/// The key a space's queued kept writes hold (`ContentStore::kept_row` settles it).
fn space_key(space: &[u8; 32]) -> super::write_back::Key {
    super::write_back::Key::of(&[b"kept facts", space])
}

impl super::write_back::Gather for KeptGather {
    fn keys(&self) -> Vec<(super::write_back::Key, Option<super::write_back::Held>)> {
        self.spaces.keys().map(|s| (space_key(s), None)).collect()
    }
    fn held(&self, _: &super::write_back::Key) -> Option<super::write_back::Held> {
        None
    }
    fn holds(&self, key: &super::write_back::Key) -> bool {
        self.spaces.keys().any(|s| space_key(s) == *key)
    }
    fn bytes(&self) -> usize {
        self.bytes
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn close(self: Box<Self>) -> Option<super::write_back::Closed> {
        let spaces: Vec<([u8; 32], Vec<KeptPut>)> = self.spaces.into_iter().map(|(s, (puts, _))| (s, puts)).filter(|(_, p)| !p.is_empty()).collect();
        if spaces.is_empty() {
            return None;
        }
        Some(super::write_back::Closed {
            what: "kept facts",
            bytes: self.bytes,
            keys: spaces.iter().map(|(s, _)| (space_key(s), None)).collect(),
            syms: super::write_back::Syms::Mapped,
            job: super::write_back::Work(super::write_back::Kind::Kept(Box::new(KeptWrite { spaces }))),
        })
    }
}

impl super::ContentStore {
    /// Writes `space`'s next kept block with `puts`, behind the run (the store's writer, in its wave's transaction):
    /// gathered with the wave's other puts, one block a space. It holds the puts' relations until their copies are
    /// made.
    pub fn queue_kept(&self, space: [u8; 32], puts: Vec<KeptPut>) {
        if puts.is_empty() {
            return;
        }
        let mut puts = Some(puts);
        if self.gather("kept facts", KeptGather::default, |g| g.add(space, puts.take().expect("the puts"))) {
            return;
        }
        // Outside a run's waves (or with no writer): its own job.
        let puts = puts.take().expect("the puts");
        let held: usize = puts.iter().map(held_bytes).sum();
        let write = KeptWrite { spaces: vec![(space, puts)] };
        self.write_back_syms(super::write_back::Syms::Mapped, "kept facts", held, vec![(space_key(&space), None)], super::write_back::Work(super::write_back::Kind::Kept(Box::new(write))));
    }

    /// The row of `space`'s kept blocks once every kept write this process queued for it is written (`settle`): what
    /// a run reads before its first wave (decision 0104), its own earlier runs' writes included.
    pub fn kept_row(&self, space: &[u8; 32]) -> StoreResult<Option<SpaceRow>> {
        self.settle(&space_key(space));
        let connection = self.connection.lock().expect("content store mutex poisoned");
        space_row(&connection, space)
    }

    /// `space`'s blocks as `row` names them, mapped.
    pub fn open_kept(&self, space: &[u8; 32], row: &SpaceRow) -> StoreResult<KeptSpace> {
        KeptSpace::open(&self.database_path, space, row)
    }

    /// The kept facts of `space` as the store's last commit has them, mapped; `None` when it keeps none.
    pub fn kept_space(&self, space: &[u8; 32]) -> StoreResult<Option<KeptSpace>> {
        let row = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            space_row(&connection, space)?
        };
        row.map(|row| KeptSpace::open(&self.database_path, space, &row)).transpose()
    }
}

/// The kept facts of the store at `from` copied beside the store at `to` (in place of any there), by the clone the
/// file system makes where it can: a store copied whole carries them, as its atoms.
pub fn copy_facts(from: &Path, to: &Path) -> std::io::Result<()> {
    let (src, dst) = (facts_dir(from), facts_dir(to));
    if dst.exists() {
        std::fs::remove_dir_all(&dst)?;
    }
    if !src.is_dir() {
        return Ok(());
    }
    for space in std::fs::read_dir(&src)? {
        let space = space?;
        let into = dst.join(space.file_name());
        private_dir(&into)?;
        for f in std::fs::read_dir(space.path())? {
            let f = f?;
            if !f.file_name().to_string_lossy().ends_with(".new") {
                clone_file(&f.path(), &into.join(f.file_name()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
