//! A space's kept facts (decisions 0401, 0402): a chain of frozen files in `<store>.facts/<space>/`, each a
//! `rerail_blocks::kept` file whose entries' rows are packed (each column in the bits its values need), mapped by every
//! process that reads the space. SQLite keeps only which files the chain has (`facts_chain`), replaced in the
//! transaction that commits a run's writes.
//!
//! The store's writer appends the space's next file behind the run (`write_space`): only what the run put, packed
//! there (0402 K3: freezing is encoding). When the chain grows long, or what its newer files replaced outweighs what it
//! still reads, the write is a roll-up instead: every live entry copied whole (one `memcpy` each) with the run's puts,
//! into one file that replaces the chain. Each file is made under a name no reader opens, frozen, renamed into place
//! and its directory synced before its row names it. The files a committed write replaced are removed by the next
//! write (`sweep`): the transaction may still roll back when this one returns, and a process that mapped one keeps
//! reading it.

use super::atoms::stored::{clone_file, new_tag, private_dir, sync_dir};
use super::{StoreError, StoreResult};
use rerail_blocks::kept::{FactKey, KeptChain, KeptFacts, KeptRead, KeptW, Meta};
use rerail_blocks::relation::{Relation, View};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Maps a forked process's atom ids to the store's, in place.
type MapIds<'a> = &'a dyn Fn(&mut [i64]) -> StoreResult<()>;

/// A put's rows as columns: the values (`n` rows of `a`, column-major), `n`, `a`, and each row's level.
type Columns = (Vec<i64>, usize, usize, Option<Vec<u32>>);

/// The files a chain holds before a write rolls it into one (0402 K4).
const MOST_FILES: usize = 8;
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
/// ids to the store's).
fn columns(what: &KeptWhat, types: &[u8], map: Option<MapIds>) -> StoreResult<Columns> {
    let (mut cols, n, a, levels) = match what {
        KeptWhat::Rows(r) => {
            let rel = r.relation();
            let (total, a) = (rel.rows.len() as usize, rel.arity as usize);
            let mut all = vec![0i64; total * a];
            for c in 0..a {
                rel.rows.read_col(c, total as u32, &mut all[c * total..(c + 1) * total]);
            }
            let view = View::full(rel);
            let dead = r.dead();
            let live = |id: usize| view.live(id as u32) && dead.get(id / 64).is_none_or(|w| w >> (id % 64) & 1 == 0);
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
        KeptWhat::Flat { widths, rows, n } => {
            let a = widths.len();
            let mut cols = vec![0i64; n * a];
            for (r, row) in rows.chunks_exact(a.max(1)).take(*n).enumerate() {
                for (c, &v) in row.iter().enumerate() {
                    cols[c * n + r] = v;
                }
            }
            (cols, *n, a, None)
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
            (rel.rows.len() as usize) * (rel.arity as usize * 8 + 8) + (rel.arity as usize + 2) * 256
        }
        KeptWhat::Flat { widths, n, .. } => n * widths.len() * 8 + (widths.len() + 2) * 256,
        KeptWhat::Reads | KeptWhat::Gone => 0,
    }
}

/// Appends `space`'s next file in transaction `t`, with `puts` (each key once), or rolls the chain and the puts into
/// one file (0402 K4). `map` maps a forked process's atom ids to the store's.
pub fn write_space(store: &Path, t: &Connection, space: &[u8; 32], puts: Vec<KeptPut>, map: Option<MapIds>) -> StoreResult<()> {
    if puts.is_empty() {
        return Ok(());
    }
    let io = |e: std::io::Error| StoreError::Io(e);
    let dir = space_dir(store, space);
    private_dir(&dir).map_err(io)?;
    let rows = chain_rows(t, space)?;
    sweep(&dir, &rows);
    let old = KeptSpace::open(store, space, &rows)?;
    let chain = old.view();
    let put_keys: HashSet<FactKey<'_>> = puts.iter().map(KeptPut::key).collect();
    // What the chain would hold that newer entries replaced, once these puts are in: a roll-up drops it.
    let replaced: usize = puts.iter().filter(|p| !matches!(p.what, KeptWhat::Reads)).filter_map(|p| chain.get(&p.key())).map(|k| k.bytes()).sum();
    let dropped = chain.superseded_bytes() + replaced;
    let roll = !old.files.is_empty() && (old.files.len() >= MOST_FILES || (dropped > ROLL_FLOOR && 2 * dropped > chain.bytes()));
    let carried: Vec<rerail_blocks::kept::Kept<'_>> = if roll { chain.all().filter(|k| !put_keys.contains(&k.key)).collect() } else { Vec::new() };
    let reads_rows: usize = puts.iter().filter(|p| matches!(p.what, KeptWhat::Reads)).filter_map(|p| chain.get(&p.key())).map(|k| k.bytes()).sum();
    let reserve = carried.iter().map(|k| k.bytes() + 64).sum::<usize>() + puts.iter().map(|p| room_of(&p.what)).sum::<usize>() + if roll { reads_rows } else { 0 } + (64 << 20);
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
    for k in &carried {
        w.copy(k, None).map_err(io)?;
    }
    for p in &puts {
        let reads: Vec<(&str, u64)> = p.reads.iter().map(|(r, d)| (r.as_str(), *d)).collect();
        let meta = Meta { key: p.key(), digest: p.digest, reads: &reads, types: &p.types };
        match &p.what {
            KeptWhat::Rows(_) | KeptWhat::Flat { .. } => {
                let (cols, n, a, levels) = columns(&p.what, &p.types, map)?;
                w.put_cols(meta, &cols, n, a, levels.as_deref()).map_err(io)?;
            }
            KeptWhat::Reads => {
                if let Some(k) = chain.get(&p.key()) {
                    let meta = Meta { types: k.types, ..meta };
                    if roll { w.copy(&k, Some(meta)).map_err(io)? } else { w.same(meta) }
                }
            }
            KeptWhat::Gone => {
                if !roll && chain.get(&p.key()).is_some() {
                    w.gone(p.key());
                }
            }
        }
    }
    w.finish().map_err(io)?;
    std::fs::rename(&making, dir.join(&name)).map_err(io)?;
    sync_dir(&dir).map_err(io)?;
    if roll {
        t.prepare_cached("DELETE FROM main.facts_chain WHERE space = ?1")?.execute([&space[..]])?;
    }
    t.prepare_cached("INSERT INTO main.facts_chain(space, seq, name, tag) VALUES (?1, ?2, ?3, ?4)")?.execute(rusqlite::params![&space[..], seq, name, tag as i64])?;
    Ok(())
}

/// A space's next file, queued for the store's writer (`ContentStore::queue_kept`).
pub(super) struct KeptWrite {
    space: [u8; 32],
    puts: Vec<KeptPut>,
}

impl KeptWrite {
    pub(super) fn run(&mut self, store: &super::ContentStore) -> StoreResult<()> {
        let puts = std::mem::take(&mut self.puts);
        let path = store.database_path.clone();
        store.immediate_transaction_once(|t| {
            // Forked (decision 0170): the puts' atom columns are mapped to the store's ids in this transaction.
            let map = |v: &mut [i64]| store.atoms.to_store(t, super::ProcessIds::of_process(v)).map(drop);
            write_space(&path, t, &self.space, puts, store.atoms.forked().then_some(&map as &dyn Fn(&mut [i64]) -> StoreResult<()>))
        })
    }
}

impl super::ContentStore {
    /// Writes `space`'s next kept file with `puts`, behind the run (the store's writer, in its wave's transaction). It
    /// holds the puts' relations until they are packed.
    pub fn queue_kept(&self, space: [u8; 32], puts: Vec<KeptPut>) {
        if puts.is_empty() {
            return;
        }
        let held: usize = puts.iter().map(|p| match &p.what {
            KeptWhat::Flat { rows, .. } => rows.len() * 8,
            _ => 0,
        }).sum::<usize>() + puts.len() * 64;
        let write = KeptWrite { space, puts };
        self.write_back_syms(super::write_back::Syms::Mapped, "kept facts", held, Vec::new(), super::write_back::Work(super::write_back::Kind::Kept(Box::new(write))));
    }

    /// The kept facts of `space` as the store's last commit has them, mapped; `None` when it keeps none.
    pub fn kept_space(&self, space: &[u8; 32]) -> StoreResult<Option<KeptSpace>> {
        let rows = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            chain_rows(&connection, space)?
        };
        if rows.is_empty() {
            return Ok(None);
        }
        KeptSpace::open(&self.database_path, space, &rows).map(Some)
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
