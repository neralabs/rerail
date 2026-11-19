//! A space's kept facts (decisions 0401, 0417): a chain of frozen files in `<store>.facts/<space>/`, each a
//! `rerail_blocks::kept` file whose entries' rows are packed (each column in the bits its values need), mapped by every
//! process that reads the space. SQLite keeps only which files the chain has (`facts_chain`), replaced in the
//! transaction that commits a run's writes.
//!
//! The store's writer appends the space's next file behind the run (`write_space`): what the run put, packed there
//! (0417 K3: freezing is encoding), merged with the chain's newest files where they weigh no more than twice what
//! joins them, so the files' sizes grow geometrically; or with the whole chain, when what its newer entries replaced
//! outweighs what it still reads. A merged entry is copied whole (one `memcpy`). Each file is made under a name no reader opens, frozen, renamed into place
//! and its directory synced before its row names it. The files a committed write replaced are removed by the next
//! write (`sweep`): the transaction may still roll back when this one returns, and a process that mapped one keeps
//! reading it.

use super::atoms::stored::{clone_file, new_tag, private_dir, sync_dir};
use super::{StoreError, StoreResult};
use rerail_blocks::kept::{FactKey, KeptChain, KeptFacts, KeptRead, KeptW, Meta, Newest};
use rerail_blocks::relation::{Relation, View};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Maps a forked process's atom ids to the store's, in place.
type MapIds<'a> = &'a dyn Fn(&mut [i64]) -> StoreResult<()>;

/// A put's rows as columns: the values (`n` rows of `a`, column-major), `n`, `a`, and each row's level.
type Columns = (Vec<i64>, usize, usize, Option<Vec<u32>>);

/// The bytes of replaced rows under which a chain is not rolled up for them: a roll-up then saves less than it costs.
const ROLL_FLOOR: usize = 1 << 20;

/// A relation the keep puts, lent by its holder (the engine's run): its rows, dead bits and levels, kept alive while
/// this lives. Nothing writes them meanwhile.
pub trait KeptRows: Send + Sync {
    fn relation(&self) -> &Relation;
    fn dead(&self) -> &[u64];
    fn levels(&self) -> &[u32];
}

/// What a put is.
pub enum KeptWhat {
    /// A relation the run derived: its live rows, packed.
    Rows(Box<dyn KeptRows>),
    /// Rows not held as a relation (an input's granule): `n` rows of `widths.len()` columns, flat; each row's level
    /// (decision 0090) where its relation has them. `given`: kept in the order given, every row (an input's granule
    /// or documents' table, read by position); else in canonical order, as a relation's.
    Flat { widths: Vec<u32>, rows: Vec<i64>, n: usize, levels: Option<Vec<u32>>, given: bool },
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

pub fn space_dir(store: &Path, space: &[u8; 32]) -> PathBuf {
    facts_dir(store).join(space.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// A space's chain as its rows name it, oldest first: (seq, name, tag).
pub type ChainRows = Vec<(i64, String, u64)>;

/// The rows of `space`'s chain; empty when it keeps no facts.
pub fn chain_rows(connection: &Connection, space: &[u8; 32]) -> StoreResult<ChainRows> {
    let mut st = connection.prepare_cached("SELECT seq, name, tag FROM main.facts_chain WHERE space = ?1 ORDER BY seq")?;
    let rows = st.query_map([&space[..]], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A space's files, mapped, oldest first.
#[derive(Default)]
pub struct KeptSpace {
    pub files: Vec<KeptFacts>,
}

impl KeptSpace {
    /// The files `rows` name, mapped, each checked against its tag.
    pub fn open(store: &Path, space: &[u8; 32], rows: &ChainRows) -> StoreResult<KeptSpace> {
        let dir = space_dir(store, space);
        let files = rows
            .iter()
            .map(|(_, name, tag)| KeptFacts::open(&dir.join(name), *tag).map_err(|e| StoreError::Corrupt(format!("a space's kept facts: {e}"))))
            .collect::<StoreResult<_>>()?;
        Ok(KeptSpace { files })
    }

    /// The space as one.
    pub fn view(&self) -> KeptChain<'_> {
        KeptChain { files: &self.files }
    }
}

/// Removes the files of `dir` that `rows` (read in the write transaction) do not name: the files a committed write
/// replaced, a rolled-back write's, a crashed maker's under its making name. A process mapping one keeps reading it.
fn sweep(dir: &Path, rows: &ChainRows) {
    let named: HashSet<&str> = rows.iter().map(|(_, n, _)| n.as_str()).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !named.contains(name.as_str()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A put's rows as columns (`n` rows of `a`, column-major) and their levels: a relation's live rows (neither its own
/// dead nor the run's), or a granule's flat rows; atom columns (`types` `s`) mapped through `map` (a forked process's
/// ids to the store's). A continuation (decision 0384 steps 7-8) puts every row it reads: its chain's first, read
/// down the chain (`Relation::get`), then its own.
fn columns(what: &KeptWhat, types: &[u8], map: Option<MapIds>) -> StoreResult<Columns> {
    let (mut cols, n, a, levels) = match what {
        KeptWhat::Rows(r) => {
            let rel = r.relation();
            let (total, a) = (rel.len_all() as usize, rel.arity as usize);
            let mut all = vec![0i64; total * a];
            for c in 0..a {
                let col = &mut all[c * total..(c + 1) * total];
                if rel.continues() {
                    for (id, v) in col.iter_mut().enumerate() {
                        *v = rel.get(id as u32, c);
                    }
                } else {
                    rel.rows.read_col(c, total as u32, col);
                }
            }
            // A continuation's ids are all its own to kill (its dead bits cover its chain's rows it reads); a full
            // view of one is not taken (its ids below its base's rows are its base's).
            let view = (!rel.continues()).then(|| View::full(rel));
            let dead = r.dead();
            let own_live = |id: u32| view.as_ref().map_or_else(|| !rel.dead_bit(id), |v| v.live(id));
            let live = |id: usize| own_live(id as u32) && dead.get(id / 64).is_none_or(|w| w >> (id % 64) & 1 == 0);
            let levels = r.levels();
            let levels = (levels.len() == total && total > 0).then_some(levels);
            if (0..total).all(live) {
                (all, total, a, levels.map(<[u32]>::to_vec))
            } else {
                let ids: Vec<usize> = (0..total).filter(|&id| live(id)).collect();
                let n = ids.len();
                let mut cols = vec![0i64; n * a];
                for c in 0..a {
                    for (o, &id) in cols[c * n..(c + 1) * n].iter_mut().zip(&ids) {
                        *o = all[c * total + id];
                    }
                }
                (cols, n, a, levels.map(|l| ids.iter().map(|&id| l[id]).collect()))
            }
        }
        KeptWhat::Flat { widths, rows, n, levels, .. } => {
            let a = widths.len();
            let mut cols = vec![0i64; n * a];
            for (r, row) in rows.chunks_exact(a.max(1)).take(*n).enumerate() {
                for (c, &v) in row.iter().enumerate() {
                    cols[c * n + r] = v;
                }
            }
            (cols, *n, a, levels.clone().filter(|l| l.len() == *n))
        }
        KeptWhat::Reads | KeptWhat::Gone => unreachable!("a put with rows"),
    };
    if let Some(map) = map {
        for (c, _) in types.iter().enumerate().filter(|(_, t)| **t == b's').take_while(|(c, _)| *c < a) {
            map(&mut cols[c * n..(c + 1) * n])?;
        }
    }
    Ok((cols, n, a, levels))
}

/// The most bytes a put's packed rows take (8 a value, the sort's duplicates included), and their descriptors.
fn room_of(what: &KeptWhat) -> usize {
    match what {
        KeptWhat::Rows(r) => {
            let rel = r.relation();
            (rel.len_all() as usize) * (rel.arity as usize * 8 + 8) + (rel.arity as usize + 2) * 256
        }
        KeptWhat::Flat { widths, n, .. } => n * widths.len() * 8 + (widths.len() + 2) * 256,
        KeptWhat::Reads | KeptWhat::Gone => 0,
    }
}

/// What a write weighs at least when choosing the files it merges: small files are copied into the next write, so a
/// chain of small commits stays one small file, not one file a commit (each file costs a header and a lookup).
const MERGE_FLOOR: usize = 1 << 16;

/// The bytes a put's rows are expected to pack into: about 3 bytes a value (ep2's kept facts, 0417 step 4). Weighs
/// the new file against the chain's newest ones; never sizes anything.
fn expected_bytes(what: &KeptWhat) -> usize {
    match what {
        KeptWhat::Rows(r) => r.relation().len_all() as usize * r.relation().arity as usize * 3,
        KeptWhat::Flat { widths, n, .. } => n * widths.len() * 3,
        KeptWhat::Reads | KeptWhat::Gone => 0,
    }
}

/// Appends `space`'s next file in transaction `t`, with `puts` (each key once), merged with the chain's newest files
/// where they weigh no more than twice what joins them (0417 K4: sizes grow geometrically, so a chain stays
/// logarithmic in its bytes and a value is copied a logarithmic number of times), or with the whole chain when what
/// it holds that newer entries replaced outweighs the rest. `map` maps a forked process's atom ids to the store's.
/// The chain's rows after it, for this process's snapshot (`held`); `None` when nothing was put.
pub fn write_space(store: &Path, t: &Connection, space: &[u8; 32], puts: Vec<KeptPut>, map: Option<MapIds>) -> StoreResult<Option<ChainRows>> {
    if puts.is_empty() {
        return Ok(None);
    }
    let io = |e: std::io::Error| StoreError::Io(e);
    let dir = space_dir(store, space);
    private_dir(&dir).map_err(io)?;
    let rows = chain_rows(t, space)?;
    sweep(&dir, &rows);
    let old = KeptSpace::open(store, space, &rows)?;
    let chain = old.view();
    let put_keys: HashSet<FactKey<'_>> = puts.iter().map(KeptPut::key).collect();
    // The files merged into this one: the newest ones, each weighing no more than twice the bytes joining it.
    let mut from = old.files.len();
    let mut joining: usize = puts.iter().map(|p| expected_bytes(&p.what)).sum::<usize>().max(MERGE_FLOOR);
    while from > 0 && old.files[from - 1].bytes() <= 2 * joining {
        joining += old.files[from - 1].bytes();
        from -= 1;
    }
    // Or every file, when what the chain would hold that newer entries replaced outweighs what it reads.
    let replaced: usize = puts.iter().filter(|p| !matches!(p.what, KeptWhat::Reads)).filter_map(|p| chain.get(&p.key())).map(|k| k.bytes()).sum();
    let dropped = chain.superseded_bytes() + replaced;
    if dropped > ROLL_FLOOR && 2 * dropped > chain.bytes() {
        from = 0;
    }
    let (older, merged) = (KeptChain { files: &old.files[..from] }, KeptChain { files: &old.files[from..] });
    let carried: Vec<(FactKey<'_>, Newest<'_>)> = merged.newest().into_iter().filter(|(k, _)| !put_keys.contains(k)).collect();
    let carried_bytes: usize = carried.iter().map(|(_, n)| if let Newest::Kept(k) = n { k.bytes() + 64 } else { 0 }).sum();
    let reads_rows: usize = puts.iter().filter(|p| matches!(p.what, KeptWhat::Reads)).filter_map(|p| chain.get(&p.key())).map(|k| k.bytes()).sum();
    let reserve = carried_bytes + reads_rows + puts.iter().map(|p| room_of(&p.what)).sum::<usize>() + (64 << 20);
    let tag = loop {
        let t = new_tag();
        if t != 0 {
            break t;
        }
    };
    let seq = rows.last().map_or(1, |r| r.0 + 1);
    let name = format!("c{seq}-{tag:016x}.blk");
    let making = dir.join(format!("{name}.new"));
    let mut w = KeptW::create(&making, tag, reserve).map_err(io)?;
    // What the merged files held: rows copied whole; a gone or reads-only entry kept while an older file holds the key.
    for (key, n) in &carried {
        match n {
            Newest::Kept(k) => w.copy(k, None).map_err(io)?,
            Newest::Gone => {
                if older.get(key).is_some() {
                    w.gone(*key);
                }
            }
            Newest::Same(same) => {
                if older.get(key).is_some() {
                    let reads: Vec<(&str, u64)> = same.reads().collect();
                    w.same(Meta { key: *key, digest: same.digest, reads: &reads, types: same.types });
                }
            }
        }
    }
    for p in &puts {
        let reads: Vec<(&str, u64)> = p.reads.iter().map(|(r, d)| (r.as_str(), *d)).collect();
        let meta = Meta { key: p.key(), digest: p.digest, reads: &reads, types: &p.types };
        match &p.what {
            KeptWhat::Rows(_) | KeptWhat::Flat { .. } => {
                let (cols, n, a, levels) = columns(&p.what, &p.types, map)?;
                let given = if matches!(p.what, KeptWhat::Flat { given: true, .. }) { rerail_blocks::packed::Order::AsGiven } else { rerail_blocks::packed::Order::Canonical };
                w.put_cols(meta, &cols, n, a, levels.as_deref(), given).map_err(io)?;
            }
            KeptWhat::Reads => {
                if let Some(k) = chain.get(&p.key()) {
                    // Its rows' digest and types are the entry's: a retag changes the reads alone (its put has
                    // none; a digest of 0 left every retagged relation unbound, ep2's k1 deriving 877 strata again).
                    let meta = Meta { types: k.types, digest: k.digest, ..meta };
                    // Rows in a merged file are copied with the new reads; rows in an older one stay there.
                    if merged.files.iter().any(|f| f.holds(k.rows)) { w.copy(&k, Some(meta)).map_err(io)? } else { w.same(meta) }
                }
            }
            KeptWhat::Gone => {
                if older.get(&p.key()).is_some() {
                    w.gone(p.key());
                }
            }
        }
    }
    w.finish().map_err(io)?;
    std::fs::rename(&making, dir.join(&name)).map_err(io)?;
    sync_dir(&dir).map_err(io)?;
    if from < rows.len() {
        t.prepare_cached("DELETE FROM main.facts_chain WHERE space = ?1 AND seq >= ?2")?.execute(rusqlite::params![&space[..], rows[from].0])?;
    }
    t.prepare_cached("INSERT INTO main.facts_chain(space, seq, name, tag) VALUES (?1, ?2, ?3, ?4)")?.execute(rusqlite::params![&space[..], seq, &name, tag as i64])?;
    let mut now = rows[..from].to_vec();
    now.push((seq, name, tag));
    Ok(Some(now))
}

/// Spaces' next files, queued for the store's writer (`ContentStore::queue_kept`): each space's puts of a wave
/// together, written as one file in the wave's transaction.
pub(super) struct KeptWrite {
    spaces: Vec<([u8; 32], Vec<KeptPut>)>,
}

impl KeptWrite {
    pub(super) fn run(&mut self, store: &super::ContentStore) -> StoreResult<()> {
        let spaces = std::mem::take(&mut self.spaces);
        let path = store.database_path.clone();
        let written = store.immediate_transaction_once(|t| {
            // Forked (decision 0170): the puts' atom columns are mapped to the store's ids in this transaction.
            let map = |v: &mut [i64]| store.atoms.to_store(t, super::ProcessIds::of_process(v)).map(drop);
            let mut written = Vec::new();
            for (space, puts) in spaces {
                if let Some(row) = write_space(&path, t, &space, puts, store.atoms.forked().then_some(&map as &dyn Fn(&mut [i64]) -> StoreResult<()>))? {
                    written.push((space, row));
                }
            }
            Ok(written)
        })?;
        // Committed: the chains this process holds are the new ones (where it holds the store's at all).
        super::held::snapshot_kept(&path, |kept| {
            if let Some(rows) = kept {
                rows.extend(written);
            }
        });
        Ok(())
    }
}

/// A run's wave's kept puts, by space (`ContentStore::gather`): one file a space a wave, where a file a put made each
/// write sync its file and directory, and merge the chain's newest files more often (with 0401's blocks, a cold
/// data-layer run over 50 files: 2,442 blocks, 15.7 s of the writer's CPU and most of the run's wall, its queue full at
/// 300 files).
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
    /// rows an earlier put of this wave gave it, and their digest, with its reads. A retag has no digest of its own
    /// (its rows are the entry's): taken over, it gave a relation handed off with its rows (its digest known then) and
    /// retagged at the run's end digest 0, and every reader of it ran again in the next run.
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

/// The key a space's queued kept writes hold (`ContentStore::kept_chain` settles it).
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
    /// Writes `space`'s next kept file with `puts`, behind the run (the store's writer, in its wave's transaction):
    /// gathered with the wave's other puts, one file a space. It holds the puts' relations until they are packed.
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

    /// The rows of `space`'s chain once every kept write this process queued for it is written (`settle`): what a run
    /// reads before its first wave (decision 0104), its own earlier runs' writes included. `None` when it keeps none.
    pub fn kept_chain(&self, space: &[u8; 32]) -> StoreResult<Option<ChainRows>> {
        self.settle(&space_key(space));
        if let Some(Some(rows)) = super::held::snapshot_kept(&self.database_path, |kept| kept.as_ref().map(|chains| chains.get(space).cloned())) {
            return Ok(rows);
        }
        let rows = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            chain_rows(&connection, space)?
        };
        Ok((!rows.is_empty()).then_some(rows))
    }

    /// The kept facts of `space` as the store's last commit has them (this process's queued writes to it written
    /// first), mapped; `None` when it keeps none.
    pub fn kept_space(&self, space: &[u8; 32]) -> StoreResult<Option<KeptSpace>> {
        self.kept_chain(space)?.map(|rows| self.open_kept(space, &rows)).transpose()
    }

    /// Every space's chain, read once a process before a run's first wave (decision 0104): from then on
    /// `kept_chain` answers from memory, and this process's writer keeps it current. Returns the spaces read now
    /// (none when an earlier run of the process read them).
    pub fn preload_kept(&self) -> StoreResult<usize> {
        // Held under the snapshot's lock while read: this process's writer, committing meanwhile, replaces its rows
        // after this read, not before it.
        super::held::snapshot_kept(&self.database_path, |kept| self.read_kept_chains(kept)).unwrap_or(Ok(0))
    }

    fn read_kept_chains(&self, kept: &mut Option<HashMap<[u8; 32], ChainRows>>) -> StoreResult<usize> {
        if kept.is_some() {
            return Ok(0);
        }
        let mut chains: HashMap<[u8; 32], ChainRows> = HashMap::new();
        {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare("SELECT space, seq, name, tag FROM main.facts_chain ORDER BY space, seq")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)? as u64)))?;
            for row in rows {
                let (space, seq, name, tag) = row?;
                if let Ok(space) = <[u8; 32]>::try_from(space.as_slice()) {
                    chains.entry(space).or_default().push((seq, name, tag));
                }
            }
        }
        let n = chains.len();
        *kept = Some(chains);
        Ok(n)
    }

    /// `space`'s chain as `rows` name it, mapped.
    pub fn open_kept(&self, space: &[u8; 32], rows: &ChainRows) -> StoreResult<KeptSpace> {
        KeptSpace::open(&self.database_path, space, rows)
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

pub(in crate::content_store) mod vtab;

#[cfg(test)]
mod tests;
