//! A space's kept facts (decisions 0401, 0456): a chain of frozen files in `<store>.facts/<space>/`, each a
//! `rerail_blocks::kept` file whose entries' rows are packed (each column in the bits its values need), mapped by every
//! process that reads the space. SQLite keeps only which files the chain has (`facts_chain`), replaced in the
//! transaction that commits a run's writes.
//!
//! The store's writer appends the space's next file behind the run (`write_space`): what the run put, packed there
//! (0456 K3: freezing is encoding), merged with the chain's newest files where they weigh no more than twice what
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
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Maps a forked process's atom ids to the store's, in place.
type MapIds<'a> = &'a dyn Fn(&mut [i64]) -> StoreResult<()>;

/// A put's rows as columns: the values (`n` rows of `a`, column-major), `n`, `a`, and each row's level.

/// The bytes of replaced rows under which a chain is not rolled up for them: a roll-up then saves less than it costs.
const ROLL_FLOOR: usize = 1 << 20;

/// A relation the keep puts, lent by its holder (the engine's run): its rows, dead bits and levels, kept alive while
/// this lives. Nothing writes them meanwhile.
pub trait KeptRows: Send + Sync {
    fn relation(&self) -> &Relation;
    fn dead(&self) -> &[u64];
    fn levels(&self) -> &[u32];
}

/// Rows a put reads where they lie (an input's granule, read from the input in place; a view's rows, read from its
/// source; rows written in a block of their own, `BlockFlat`): `n` rows of `width` columns, which `fill` writes
/// column-major into the writer's scratch (`KeptW::put_filled`), with each row's level where `has_levels`. `given`:
/// kept in the order given, every row (read by position); else in canonical order, as a relation's.
pub trait KeptPicked: Send + Sync {
    fn rows(&self) -> usize;
    fn width(&self) -> usize;
    fn given(&self) -> bool {
        true
    }
    fn has_levels(&self) -> bool {
        false
    }
    /// The bytes it holds until written (a block of its own), not what it lends.
    fn held(&self) -> usize {
        0
    }
    fn fill(&self, cols: &mut [i64], levels: Option<&mut [u32]>);
}

/// Rows written in a block of their own, never on the heap (blocks are never copied, decision 0353): `n` rows of
/// `width` 8-byte values, column-major, as the writer reads them (`KeptPicked`). An entry's small rows (a documents'
/// table, statistics' words, a digest's entry) are made here as they are found.
pub struct BlockFlat {
    block: rerail_blocks::block::Block,
    at: usize,
    n: usize,
    width: usize,
    levels: Option<usize>,
    given: bool,
}

impl BlockFlat {
    /// `n` rows of `width` columns, every value 0, kept as given (`given`) or in canonical order.
    pub fn new(n: usize, width: usize, given: bool) -> BlockFlat {
        Self::with_levels(n, width, given, false)
    }

    /// `new`, with each row's level (0 until `put_level`) where `levels`.
    pub fn with_levels(n: usize, width: usize, given: bool, levels: bool) -> BlockFlat {
        let values = n * width * 8;
        let level_bytes = if levels { n * 4 } else { 0 };
        let block = rerail_blocks::block::Block::anon(rerail_blocks::block::kind::FACTS, values + level_bytes + 4096);
        let at = block.alloc(values.max(8), 8);
        let levels = levels.then(|| block.alloc(level_bytes.max(4), 4));
        BlockFlat { block, at, n, width, levels, given }
    }

    /// One row of these values, kept as given.
    pub fn of_row(row: &[i64]) -> BlockFlat {
        Self::of_rows(row, row.len(), true)
    }

    /// `bytes` as one column of words, kept as given: their length, then the bytes, 8 a word, little-endian, the
    /// last word's high bytes zero.
    pub fn of_bytes(bytes: &[u8]) -> BlockFlat {
        let mut f = Self::new(1 + bytes.len().div_ceil(8), 1, true);
        f.put(0, 0, bytes.len() as i64);
        for (i, c) in bytes.chunks(8).enumerate() {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            f.put(1 + i, 0, i64::from_le_bytes(w));
        }
        f
    }

    /// One row of values, as given, row-major: `rows.len() / width` rows.
    pub fn of_rows(rows: &[i64], width: usize, given: bool) -> BlockFlat {
        let n = rows.len().checked_div(width).unwrap_or(0);
        let mut f = Self::new(n, width, given);
        for (r, row) in rows.chunks_exact(width.max(1)).take(n).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                f.put(r, c, v);
            }
        }
        f
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn cols(&self) -> &[i64] {
        // SAFETY: `n * width` values allocated at `at`, written only through `&mut self`.
        unsafe { std::slice::from_raw_parts(self.block.ptr::<i64>(self.at), self.n * self.width) }
    }

    fn cols_mut(&mut self) -> &mut [i64] {
        // SAFETY: as `cols`; `&mut self` is the only writer.
        unsafe { std::slice::from_raw_parts_mut(self.block.ptr::<i64>(self.at), self.n * self.width) }
    }

    fn level_words(&self) -> Option<&[u32]> {
        // SAFETY: `n` levels allocated at `l`.
        self.levels.map(|l| unsafe { std::slice::from_raw_parts(self.block.ptr::<u32>(l), self.n) })
    }

    pub fn put(&mut self, r: usize, c: usize, v: i64) {
        let n = self.n;
        self.cols_mut()[c * n + r] = v;
    }

    pub fn get(&self, r: usize, c: usize) -> i64 {
        self.cols()[c * self.n + r]
    }

    /// A writer of its values row by row, each row's in column order, from row 0 on (`FlatWriter`).
    pub fn writer(&mut self) -> FlatWriter<'_> {
        FlatWriter { flat: self, at: 0 }
    }

    pub fn put_level(&mut self, r: usize, level: u32) {
        if let Some(l) = self.levels {
            // SAFETY: `n` levels allocated at `l`, `r` below `n` (checked).
            assert!(r < self.n);
            unsafe { *self.block.ptr::<u32>(l).add(r) = level };
        }
    }
}

/// Values written into a `BlockFlat` row by row (`BlockFlat::writer`): the `k`-th value pushed is row `k / width`'s
/// column `k % width`.
pub struct FlatWriter<'a> {
    flat: &'a mut BlockFlat,
    at: usize,
}

impl FlatWriter<'_> {
    pub fn push(&mut self, v: i64) {
        let w = self.flat.width;
        self.flat.put(self.at / w, self.at % w, v);
        self.at += 1;
    }

    pub fn extend(&mut self, vs: impl IntoIterator<Item = i64>) {
        for v in vs {
            self.push(v);
        }
    }

    /// The values written: `rows * width` when every row is whole.
    pub fn written(&self) -> usize {
        self.at
    }
}

impl KeptPicked for BlockFlat {
    fn rows(&self) -> usize {
        self.n
    }
    fn width(&self) -> usize {
        self.width
    }
    fn given(&self) -> bool {
        self.given
    }
    fn has_levels(&self) -> bool {
        self.levels.is_some()
    }
    fn held(&self) -> usize {
        self.n * (self.width * 8 + if self.levels.is_some() { 4 } else { 0 })
    }
    fn fill(&self, cols: &mut [i64], levels: Option<&mut [u32]>) {
        cols.copy_from_slice(self.cols());
        if let (Some(out), Some(l)) = (levels, self.level_words()) {
            out.copy_from_slice(l);
        }
    }
}

/// What a put is.
pub enum KeptWhat {
    /// A relation the run derived: its live rows, packed.
    Rows(Box<dyn KeptRows>),
    /// Rows read where they lie (`KeptPicked`): an input's granule, a view's rows, rows in a block of their own.
    Picked(Box<dyn KeptPicked>),
    /// A documents' table (`document_places`): its rows as the run gives them (`DOCUMENT_WORDS` each), kept in that
    /// order with each document's places, which the writer finds as it writes them (decision 0494 step 4 part 2).
    /// `tape`: the grammar its tapes are found in (grammar id, grammar version, tape format version).
    Documents { rows: BlockFlat, tape: Option<[i64; 3]> },
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

/// The relation a space's reader records are kept under (decision 0401 step 8, F4): an entry a program (its key, the
/// run's `prepared_key`), its rows the identities of the entries that program asked or put in the space, four words
/// each. A record only grows: the writer adds a put of one to the record kept (`write_space`). Collection keeps what
/// a live program's record names (`collect_kept_facts`). No relation's name starts with a NUL.
pub const READERS: &str = "\0readers";

/// The relation a partition's holder is kept under (t33 step 3): one row, the SHA-256 key of the source that holds
/// the space's entries of that partition, and the plan they were made under (zeros for none), four words each.
/// Collection drops a held partition once the store keeps no source of that key, or once its plan is dead
/// (`collect_kept_facts`), as the facts memo's rows died with their source or their plan: a memo entry kept as kept
/// facts lives exactly as long as the memo row it replaced would have.
///
/// A holder is put with its partition's whole entry: the writer drops what else the partition kept (`write_chain`),
/// and a wave's earlier puts of it (`KeptGather::add`), so its user writes an entry without reading what it was.
pub const HOLDER: &str = "\0holder";

fn holder_ident() -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(b"rerail.kept-facts/holder/v1").into()
}

/// The put that makes the source of SHA-256 key `source`, and `plan`, partition `part`'s holder in its space.
pub fn holder_put(part: i64, source: [u8; 32], plan: Option<[u8; 32]>) -> KeptPut {
    let mut row = BlockFlat::new(1, 8, true);
    for (c, w) in source.as_chunks::<8>().0.iter().chain(plan.unwrap_or([0; 32]).as_chunks::<8>().0).enumerate() {
        row.put(0, c, i64::from_le_bytes(*w));
    }
    let digest = u64::from_le_bytes(source[..8].try_into().expect("8 bytes")) ^ plan.map_or(0, |p| u64::from_le_bytes(p[..8].try_into().expect("8 bytes")));
    KeptPut { rel: HOLDER.into(), ident: holder_ident(), part, digest, reads: Vec::new(), types: b"iiiiiiii".to_vec(), what: KeptWhat::Picked(Box::new(row)) }
}

/// The source key and plan a holder entry names.
fn holder_of(entry: &rerail_blocks::kept::Kept<'_>) -> Option<([u8; 32], Option<[u8; 32]>)> {
    if entry.rows.len() != 1 || entry.types.len() != 8 {
        return None;
    }
    let digest = |at: usize| {
        let mut d = [0u8; 32];
        for (c, w) in d.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            *w = entry.rows.get(0, at + c).to_le_bytes();
        }
        d
    };
    let plan = digest(4);
    Some((digest(0), (plan != [0; 32]).then_some(plan)))
}

/// A reader record's put: program `key`'s `idents`, sorted, as rows of four words, digested.
fn readers_put(key: [u8; 32], idents: &BTreeSet<[u8; 32]>) -> KeptPut {
    use sha2::Digest as _;
    let mut rows = BlockFlat::new(idents.len(), 4, true);
    let mut w = rows.writer();
    let mut hash = sha2::Sha256::new();
    for id in idents {
        hash.update(id);
        w.extend(id.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)));
    }
    let digest = u64::from_le_bytes(hash.finalize()[..8].try_into().expect("8 bytes"));
    KeptPut { rel: READERS.into(), ident: key, part: 0, digest, reads: Vec::new(), types: b"iiii".to_vec(), what: KeptWhat::Picked(Box::new(rows)) }
}

/// The identities a kept reader record names.
pub fn record_idents<'a>(record: &'a rerail_blocks::kept::Kept<'_>) -> impl Iterator<Item = [u8; 32]> + 'a {
    (0..record.rows.len()).map(|i| {
        let mut id = [0u8; 32];
        for (c, w) in id.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            *w = record.rows.get(i, c).to_le_bytes();
        }
        id
    })
}

/// The identities a reader record's put names (`readers_put`): its rows, column-major as a picked put fills them.
fn put_idents(p: &KeptPut) -> Vec<[u8; 32]> {
    let KeptWhat::Picked(rows) = &p.what else { return Vec::new() };
    let n = rows.rows();
    let mut cols = vec![0i64; n * 4];
    rows.fill(&mut cols, None);
    (0..n).map(|r| {
        let mut id = [0u8; 32];
        for (c, w) in id.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            *w = cols[c * n + r].to_le_bytes();
        }
        id
    })
    .collect()
}

/// What a run's program asked or put of each space's entries (decision 0401 step 8), bound in the run's context
/// (`KeptReader`) by a run whose program the store keeps: recorded in the space as the program's reader record, in
/// the file of the puts that name them or, for what it only asked, at the run's end (`flush`), before its manifest.
pub struct ReaderLog {
    key: [u8; 32],
    spaces: std::sync::Mutex<HashMap<[u8; 32], Asked>>,
}

/// A space's identities a run's program asked or put: those its record holds or will (`known`, the stored record's
/// read at the first ask), and those asked that no write records yet.
#[derive(Default)]
struct Asked {
    seeded: bool,
    known: HashSet<[u8; 32]>,
    pending: BTreeSet<[u8; 32]>,
}

/// The run's reader log, in its context (decision 0442: the context goes with its jobs); none for a run whose
/// program no manifest will name.
#[derive(Clone, Default)]
pub struct KeptReader(pub Option<std::sync::Arc<ReaderLog>>);

impl ReaderLog {
    pub fn new(key: [u8; 32]) -> ReaderLog {
        ReaderLog { key, spaces: Default::default() }
    }

    /// `ident` asked in `space`; `stored`: the program's record as the space keeps it, read at a space's first ask.
    pub fn asked(&self, space: &[u8; 32], ident: &[u8; 32], stored: impl FnOnce(&[u8; 32]) -> Vec<[u8; 32]>) {
        let mut spaces = self.spaces.lock().unwrap_or_else(|e| e.into_inner());
        let a = spaces.entry(*space).or_default();
        if !a.seeded {
            a.seeded = true;
            a.known.extend(stored(&self.key));
        }
        if a.known.insert(*ident) {
            a.pending.insert(*ident);
        }
    }

    /// The record's put to write with `idents` put in `space`: those and what was asked, when any is new.
    fn put(&self, space: &[u8; 32], idents: impl Iterator<Item = [u8; 32]>) -> Option<KeptPut> {
        let mut spaces = self.spaces.lock().unwrap_or_else(|e| e.into_inner());
        let a = spaces.entry(*space).or_default();
        let mut new = std::mem::take(&mut a.pending);
        new.extend(idents.filter(|id| a.known.insert(*id)));
        (!new.is_empty()).then(|| readers_put(self.key, &new))
    }

    /// What was asked and no write recorded, queued for each space: before the run's manifest, which names the key.
    pub fn flush(&self, store: &super::ContentStore) {
        let pending: Vec<([u8; 32], BTreeSet<[u8; 32]>)> = {
            let mut spaces = self.spaces.lock().unwrap_or_else(|e| e.into_inner());
            spaces.iter_mut().filter(|(_, a)| !a.pending.is_empty()).map(|(s, a)| (*s, std::mem::take(&mut a.pending))).collect()
        };
        for (space, idents) in pending {
            store.queue_kept(space, vec![readers_put(self.key, &idents)]);
        }
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
        Self::mapped(store, space, rows).map_err(|(path, e)| StoreError::Corrupt(format!("a space's kept facts: {}: {e}", path.display())))
    }

    /// `open`'s files, or the first that does not open, with why.
    fn mapped(store: &Path, space: &[u8; 32], rows: &ChainRows) -> Result<KeptSpace, (PathBuf, std::io::Error)> {
        let dir = space_dir(store, space);
        let files = rows
            .iter()
            .map(|(_, name, tag)| {
                let path = dir.join(name);
                KeptFacts::open(&path, *tag).map_err(|e| (path, e))
            })
            .collect::<Result<_, _>>()?;
        Ok(KeptSpace { files })
    }

    /// The space as one.
    pub fn view(&self) -> KeptChain<'_> {
        KeptChain { files: &self.files }
    }
}

/// Removes the files of `dir` that `rows` (read in the write transaction) do not name: the files a committed write
/// replaced, a rolled-back write's, a crashed maker's under its making name. A process mapping one keeps reading it.
/// A file this process staged (`KeptStage`) is kept until a chain row names it: its wave has not committed yet.
fn sweep(dir: &Path, rows: &ChainRows) {
    let named: HashSet<&str> = rows.iter().map(|(_, n, _)| n.as_str()).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let staging = staging();
    let mut pending = staging.0.lock().expect("the staged kept files");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if named.contains(name.as_str()) {
            pending.remove(&path);
        } else if !pending.contains(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The kept files this process wrote ahead of their wave's commit (`KeptStage`), by path: until a chain row names
/// one, no sweep removes it. Another process's (or a crashed one's) are removed as any unnamed file.
#[derive(Default)]
struct Staging(std::sync::Mutex<HashSet<PathBuf>>);

fn staging() -> std::sync::Arc<Staging> {
    rerail_prof::code_rule_dsl::logic::profile::shared::<Staging>()
}

/// A space's puts written to a file of their own ahead of their wave (`KeptStage`): what its commit links.
#[derive(Clone)]
struct StagedFile {
    space: [u8; 32],
    name: String,
    tag: u64,
}

/// Where a gather's stage job leaves its files for the gather's commit job: `None` until it ran.
type StagedSlot = std::sync::Arc<std::sync::Mutex<Option<Result<Vec<StagedFile>, String>>>>;

/// Whether `p` is written ahead of its wave (`KeptStage`): its rows are columns as they are, no atom column that a
/// forked process maps to the store's ids at its commit, and no documents' places the commit's transaction finds.
/// Not a reader record: it only grows (decision 0401 step 8), the record the chain holds joined in the commit's
/// transaction (`write_chain`); staged alone, the newest wave's record hid every earlier one's, and the next run asked
/// their entries again and wrote its record, a write in a warm run.
fn stages(p: &KeptPut) -> bool {
    p.rel != READERS && matches!(p.what, KeptWhat::Rows(_) | KeptWhat::Picked(_)) && !p.types.contains(&b's')
}

/// A kept gather's rows, written as it closes (`Gather::close`'s `now`): each space's in a file of its own, on the
/// writer's thread, outside any transaction, and let go. Its wave's commit (`KeptWrite`) only links the files into
/// their chains. Held until the wave sealed, a cold run's kept puts were 2.3 GB of the writer's queue at the cross
/// part's peak (3,089 files).
pub(super) struct KeptStage {
    spaces: Vec<([u8; 32], Vec<KeptPut>)>,
    slot: StagedSlot,
}

impl KeptStage {
    pub(super) fn run(&mut self, store: &super::ContentStore) -> StoreResult<()> {
        let spaces = std::mem::take(&mut self.spaces);
        let flush = store.connection.lock().map_or(rerail_blocks::block::Flush::Os, |c| super::connection::flush_of(&c));
        let made = spaces.into_iter().map(|(space, puts)| stage_space(&store.database_path, &space, &puts, flush)).collect::<StoreResult<Vec<_>>>();
        let outcome = made.as_ref().map(|_| ()).map_err(|e| StoreError::Invalid(format!("kept facts staged: {e}")));
        *self.slot.lock().expect("the staged slot") = Some(made.map_err(|e| e.to_string()));
        outcome
    }
}

/// `puts` (all `stages`) of `space` written to a new file of the space's directory, named by its tag alone: its
/// chain row, and so its place in the chain, is the commit's.
fn stage_space(store: &Path, space: &[u8; 32], puts: &[KeptPut], flush: rerail_blocks::block::Flush) -> StoreResult<StagedFile> {
    let io = |e: std::io::Error| StoreError::Io(e);
    let _span = rerail_prof::code_rule_dsl::logic::profile::span(|| "kept facts: a gather's rows staged".into());
    let dir = space_dir(store, space);
    private_dir(&dir).map_err(io)?;
    let tag = loop {
        let t = new_tag();
        if t != 0 {
            break t;
        }
    };
    let name = format!("s-{tag:016x}.blk");
    let path = dir.join(&name);
    staging().0.lock().expect("the staged kept files").insert(path.clone());
    let reserve = puts.iter().map(|p| room_of(&p.what)).sum::<usize>() + (64 << 20);
    let mut w = KeptW::create(&path, tag, reserve).map_err(io)?;
    for p in puts {
        let reads: Vec<(&str, u64)> = p.reads.iter().map(|(r, d)| (r.as_str(), *d)).collect();
        let meta = Meta { key: p.key(), digest: p.digest, reads: &reads, types: &p.types };
        put_rows(&mut w, meta, p, None)?;
    }
    w.finish(flush).map_err(io)?;
    sync_dir(&dir, flush).map_err(io)?;
    Ok(StagedFile { space: *space, name, tag })
}

/// Links `f` as the newest file of its space's chain, in the commit's transaction.
fn link_staged(t: &Connection, f: &StagedFile) -> StoreResult<()> {
    let seq = chain_rows(t, &f.space)?.last().map_or(1, |r| r.0 + 1);
    t.prepare_cached("INSERT INTO main.facts_chain(space, seq, name, tag) VALUES (?1, ?2, ?3, ?4)")?.execute(rusqlite::params![&f.space[..], seq, f.name, f.tag as i64])?;
    Ok(())
}

/// Writes `p`'s rows (`KeptWhat::Rows` or `Flat`) into `w`: a relation's live rows (neither its own dead nor the
/// run's) read where they lie in its block, or a granule's flat rows, each column filled straight into the packer's
/// scratch (`KeptW::put_filled`); atom columns (`types` `s`) mapped there through `map` (a forked process's ids to the
/// store's). Nothing is staged on the heap: copied into vectors first, every kept relation was a second copy of its
/// block on the writer (blocks are never copied, decision 0353).
fn put_rows(w: &mut KeptW, meta: Meta, p: &KeptPut, map: Option<MapIds>) -> StoreResult<()> {
    let given = if matches!(&p.what, KeptWhat::Picked(k) if k.given()) { rerail_blocks::packed::Order::AsGiven } else { rerail_blocks::packed::Order::Canonical };
    let types = &p.types;
    let mapped = |raw: &mut [i64], n: usize, a: usize| -> StoreResult<()> {
        if let Some(map) = map {
            for (c, _) in types.iter().enumerate().filter(|(_, t)| **t == b's').take_while(|(c, _)| *c < a) {
                map(&mut raw[c * n..(c + 1) * n])?;
            }
        }
        Ok(())
    };
    match &p.what {
        KeptWhat::Rows(r) => {
            let rel = r.relation();
            // A continuation (decision 0384 steps 7 and 8) is kept with every row it reads, its base's first: its
            // ids run down the chain, its base's dead bits read through its view (as `packed::pack_open` reads it).
            let (total, a) = (rel.len_all() as usize, rel.arity as usize);
            let view = if rel.continues() { View { base: rel.base(), top: rel } } else { View::full(rel) };
            let dead = r.dead();
            let live = |id: usize| view.live(id as u32) && dead.get(id / 64).is_none_or(|w| w >> (id % 64) & 1 == 0);
            let all_live = (0..total).all(live);
            let n = if all_live { total } else { (0..total).filter(|&id| live(id)).count() };
            let levels = r.levels();
            let levels = (levels.len() == total && total > 0).then_some(levels);
            w.put_filled(meta, n, a, levels.is_some(), given, |raw, lv| {
                for c in 0..a {
                    let col = &mut raw[c * n..(c + 1) * n];
                    if all_live && !rel.continues() {
                        rel.rows.read_col(c, total as u32, col);
                    } else {
                        for (o, id) in col.iter_mut().zip((0..total).filter(|&id| live(id))) {
                            *o = rel.get(id as u32, c);
                        }
                    }
                }
                if let (Some(out), Some(l)) = (lv, levels) {
                    for (o, id) in out.iter_mut().zip((0..total).filter(|&id| live(id))) {
                        *o = l[id];
                    }
                }
                mapped(raw, n, a)
            })?;
        }
        KeptWhat::Picked(picked) => {
            let (n, a) = (picked.rows(), picked.width());
            w.put_filled(meta, n, a, picked.has_levels() && n > 0, given, |raw, lv| {
                picked.fill(raw, lv);
                mapped(raw, n, a)
            })?;
        }
        KeptWhat::Reads | KeptWhat::Gone => unreachable!("a put with rows"),
        KeptWhat::Documents { .. } => unreachable!("a documents' table is resolved first (`write_space`)"),
    }
    Ok(())
}

/// The most bytes a put's packed rows take (8 a value, the sort's duplicates included), and their descriptors.
fn room_of(what: &KeptWhat) -> usize {
    match what {
        KeptWhat::Rows(r) => {
            let rel = r.relation();
            (rel.len_all() as usize) * (rel.arity as usize * 8 + 8) + (rel.arity as usize + 2) * 256
        }
        KeptWhat::Picked(p) => p.rows() * p.width() * 8 + (p.width() + 2) * 256,
        KeptWhat::Documents { rows, .. } => rows.len() * super::PLACED_WORDS * 8 + (super::PLACED_WORDS + 2) * 256,
        KeptWhat::Reads | KeptWhat::Gone => 0,
    }
}

/// What a write weighs at least when choosing the files it merges: small files are copied into the next write, so a
/// chain of small commits stays one small file, not one file a commit (each file costs a header and a lookup).
const MERGE_FLOOR: usize = 1 << 16;

/// The bytes a put's rows are expected to pack into: about 3 bytes a value (ep2's kept facts, 0456 step 4). Weighs
/// the new file against the chain's newest ones; never sizes anything.
fn expected_bytes(what: &KeptWhat) -> usize {
    match what {
        KeptWhat::Rows(r) => r.relation().len_all() as usize * r.relation().arity as usize * 3,
        KeptWhat::Picked(p) => p.rows() * p.width() * 3,
        KeptWhat::Documents { rows, .. } => rows.len() * super::PLACED_WORDS * 3,
        KeptWhat::Reads | KeptWhat::Gone => 0,
    }
}

/// Appends `space`'s next file in transaction `t`, with `puts` (each key once), merged with the chain's newest files
/// where they weigh no more than twice what joins them (0456 K4: sizes grow geometrically, so a chain stays
/// logarithmic in its bytes and a value is copied a logarithmic number of times), or with the whole chain when what
/// it holds that newer entries replaced outweighs the rest. `map` maps a forked process's atom ids to the store's.
pub fn write_space(store: &Path, t: &Connection, space: &[u8; 32], puts: Vec<KeptPut>, map: Option<MapIds>) -> StoreResult<Option<ChainRows>> {
    write_chain(store, t, space, puts, map, None)
}

/// Collection's roll-up of `space` in transaction `t` (decision 0401 F4): its next base, one file of the entries
/// `keep` keeps, when it keeps fewer than the space holds; with none kept, the space is dropped (its rows; its files
/// go once committed, `drop_space_files`). `None` when every entry stays.
pub fn rewrite_space(store: &Path, t: &Connection, space: &[u8; 32], keep: &dyn Fn(&FactKey<'_>) -> bool) -> StoreResult<Option<ChainRows>> {
    write_chain(store, t, space, Vec::new(), None, Some(keep))
}

/// The files of a space collection dropped, removed once its commit is: a process mapping one keeps reading it.
pub fn drop_space_files(store: &Path, space: &[u8; 32]) {
    let _ = std::fs::remove_dir_all(space_dir(store, space));
}

/// The files of `space` its committed `rows` do not name, removed (`sweep`): those a collection's roll-up replaced.
pub fn sweep_space(store: &Path, space: &[u8; 32], rows: &ChainRows) {
    sweep(&space_dir(store, space), rows);
}

/// `write_space`, or with `keep`, `rewrite_space`.
fn write_chain(store: &Path, t: &Connection, space: &[u8; 32], puts: Vec<KeptPut>, map: Option<MapIds>, keep: Option<&dyn Fn(&FactKey<'_>) -> bool>) -> StoreResult<Option<ChainRows>> {
    if puts.is_empty() && keep.is_none() {
        return Ok(None);
    }
    // A documents' table's places, found in this transaction: its rows as kept.
    let mut puts = puts
        .into_iter()
        .map(|mut p| {
            if let KeptWhat::Documents { rows, tape } = &p.what {
                let rows = super::document_places::resolve(t, rows, *tape)?;
                p.what = KeptWhat::Picked(Box::new(rows));
            }
            Ok(p)
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let io = |e: std::io::Error| StoreError::Io(e);
    let dir = space_dir(store, space);
    private_dir(&dir).map_err(io)?;
    let opened = rerail_prof::code_rule_dsl::logic::profile::span(|| "kept facts: the space's files opened".into());
    let rows = chain_rows(t, space)?;
    sweep(&dir, &rows);
    let old = KeptSpace::open(store, space, &rows)?;
    drop(opened);
    let chain = old.view();
    if rows.is_empty() && keep.is_some() {
        return Ok(None);
    }
    // A reader record only grows: a put of one is what the space keeps and what it adds (decision 0401 step 8).
    for p in puts.iter_mut().filter(|p| p.rel == READERS) {
        let Some(was) = chain.get(&p.key()) else { continue };
        let all: BTreeSet<[u8; 32]> = record_idents(&was).chain(put_idents(p)).collect();
        *p = readers_put(p.ident, &all);
    }
    // Collection's: the whole chain rewritten without what `keep` drops, when it drops any.
    if let Some(keep) = keep {
        let newest = chain.newest();
        if newest.iter().all(|(k, _)| keep(k)) {
            return Ok(None);
        }
        if !newest.iter().any(|(k, n)| keep(k) && matches!(n, Newest::Kept(_))) {
            t.prepare_cached("DELETE FROM main.facts_chain WHERE space = ?1")?.execute([&space[..]])?;
            return Ok(Some(Vec::new()));
        }
    }
    // A held partition is put whole (`HOLDER`): what else it kept goes.
    let held: BTreeSet<i64> = puts.iter().filter(|p| p.rel == HOLDER && !matches!(p.what, KeptWhat::Gone)).map(|p| p.part).collect();
    if !held.is_empty() {
        let named: HashSet<(&str, [u8; 32], i64)> = puts.iter().map(|p| (p.rel.as_str(), p.ident, p.part)).collect();
        let gone: Vec<KeptPut> = chain
            .all()
            .filter(|e| held.contains(&e.key.part) && e.key.rel != READERS && !named.contains(&(e.key.rel, *e.key.ident, e.key.part)))
            .map(|e| KeptPut { rel: e.key.rel.to_owned(), ident: *e.key.ident, part: e.key.part, digest: 0, reads: Vec::new(), types: e.types.to_vec(), what: KeptWhat::Gone })
            .collect();
        puts.extend(gone);
    }
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
    if (dropped > ROLL_FLOOR && 2 * dropped > chain.bytes()) || keep.is_some() {
        from = 0;
    }
    let (older, merged) = (KeptChain { files: &old.files[..from] }, KeptChain { files: &old.files[from..] });
    let carried: Vec<(FactKey<'_>, Newest<'_>)> = merged.newest().into_iter().filter(|(k, _)| !put_keys.contains(k) && keep.is_none_or(|keep| keep(k))).collect();
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
    let carrying = rerail_prof::code_rule_dsl::logic::profile::span(|| "kept facts: the merged files' entries carried".into());
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
    drop(carrying);
    let putting = rerail_prof::code_rule_dsl::logic::profile::span(|| "kept facts: the puts written".into());
    for p in &puts {
        let reads: Vec<(&str, u64)> = p.reads.iter().map(|(r, d)| (r.as_str(), *d)).collect();
        let meta = Meta { key: p.key(), digest: p.digest, reads: &reads, types: &p.types };
        match &p.what {
            KeptWhat::Rows(_) | KeptWhat::Picked(_) => {
                put_rows(&mut w, meta, p, map)?;
            }
            KeptWhat::Documents { .. } => unreachable!("a documents' table is resolved first (above)"),
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
    drop(putting);
    let finishing = rerail_prof::code_rule_dsl::logic::profile::span(|| "kept facts: the block finished and synced".into());
    // Written back as the store's flush mode says (decision 0481): by default the OS writes it at its own pace.
    let flush = crate::content_store::connection::flush_of(t);
    w.finish(flush).map_err(io)?;
    std::fs::rename(&making, dir.join(&name)).map_err(io)?;
    sync_dir(&dir, flush).map_err(io)?;
    drop(finishing);
    if from < rows.len() {
        t.prepare_cached("DELETE FROM main.facts_chain WHERE space = ?1 AND seq >= ?2")?.execute(rusqlite::params![&space[..], rows[from].0])?;
    }
    t.prepare_cached("INSERT INTO main.facts_chain(space, seq, name, tag) VALUES (?1, ?2, ?3, ?4)")?.execute(rusqlite::params![&space[..], seq, name, tag as i64])?;
    let mut now: ChainRows = rows[..from].to_vec();
    now.push((seq, name, tag));
    Ok(Some(now))
}

/// Spaces' next files, queued for the store's writer (`ContentStore::queue_kept`): each space's puts of a wave
/// together, appended as one file in the wave's transaction.
pub(super) struct KeptWrite {
    spaces: Vec<([u8; 32], Vec<KeptPut>)>,
    /// The files its gather's rows were staged in (`KeptStage`), linked before its own puts are written.
    staged: Option<StagedSlot>,
}

impl KeptWrite {
    pub(super) fn run(&mut self, store: &super::ContentStore) -> StoreResult<()> {
        let spaces = std::mem::take(&mut self.spaces);
        let staged: Vec<StagedFile> = match self.staged.as_ref().map(|s| s.lock().expect("the staged slot").clone()) {
            None => Vec::new(),
            Some(Some(Ok(files))) => files,
            Some(Some(Err(e))) => return Err(StoreError::Invalid(format!("kept facts staged: {e}"))),
            Some(None) => return Err(StoreError::Invalid("kept facts: the gather's rows were not staged before its wave".into())),
        };
        let path = store.database_path.clone();
        let written = store.immediate_transaction_once(|t| {
            // Forked (decision 0170): the puts' atom columns are mapped to the store's ids in this transaction.
            let map = |v: &mut [i64]| store.atoms.to_store(t, super::ProcessIds::of_process(v)).map(drop);
            for f in &staged {
                link_staged(t, f)?;
            }
            let mut written = Vec::new();
            for (space, puts) in spaces {
                if let Some(row) = write_space(&path, t, &space, puts, store.atoms.forked().then_some(&map as &dyn Fn(&mut [i64]) -> StoreResult<()>))? {
                    written.push((space, row));
                }
            }
            for f in &staged {
                if !written.iter().any(|(s, _)| *s == f.space) {
                    written.push((f.space, chain_rows(t, &f.space)?));
                }
            }
            Ok(written)
        })?;
        // Committed: the rows this process holds are the new ones (where it holds the store's rows at all).
        super::held::snapshot_kept(&path, |kept| {
            if let Some(rows) = kept {
                rows.extend(written);
            }
        });
        Ok(())
    }
}

/// A run's wave's kept puts, by space (`ContentStore::gather`): one file a space a wave, where a file a put made
/// each write weigh the chain again, and sync its file and directory (a cold data-layer run over 50
/// files: 2,442 blocks, 15.7 s of the writer's CPU and most of the run's wall, its queue full at 300 files).
/// A space's puts gathered, and each one's place among them by its key (relation, digest, part).
type Gathered = (Vec<KeptPut>, std::collections::HashMap<(String, [u8; 32], i64), usize>);

#[derive(Default)]
struct KeptGather {
    spaces: std::collections::BTreeMap<[u8; 32], Gathered>,
    bytes: usize,
}

/// The bytes a put holds until written: a flat put's rows, and a little for the rest (a relation is lent, not held).
fn held_bytes(p: &KeptPut) -> usize {
    64 + match &p.what {
        KeptWhat::Picked(k) => k.held(),
        KeptWhat::Documents { rows, .. } => rows.held(),
        _ => 0,
    }
}

impl KeptGather {
    /// `puts` of `space` folded as one wave's are (`add`): each key once.
    fn folded(space: [u8; 32], puts: Vec<KeptPut>) -> Vec<KeptPut> {
        let mut g = KeptGather::default();
        g.add(space, puts);
        g.spaces.remove(&space).map(|(list, _)| list).unwrap_or_default()
    }

    /// `space`'s puts, each key once: a later put replaces an earlier one of its key, but a retag (`Reads`) keeps the
    /// rows an earlier put of this wave gave it, and their digest, with its reads. A retag has no digest of its own
    /// (its rows are the entry's): taken over, it gave a relation handed off with its rows (its digest known then) and
    /// retagged at the run's end digest 0, and every reader of it ran again in the next run. A program's reader record
    /// joins its earlier one, as the space's record only grows (decision 0401 step 8): each names what its own puts
    /// did, and replaced, the wave's last record was the program's, so the next run asked the others' entries again,
    /// and wrote its record (a warm run's write). A held partition is put whole (`HOLDER`): its earlier puts of this
    /// wave go with it, its reader records kept.
    fn add(&mut self, space: [u8; 32], puts: Vec<KeptPut>) {
        let (list, at) = self.spaces.entry(space).or_default();
        let held: BTreeSet<i64> = puts.iter().filter(|p| p.rel == HOLDER && !matches!(p.what, KeptWhat::Gone)).map(|p| p.part).collect();
        if !held.is_empty() && list.iter().any(|p| held.contains(&p.part) && p.rel != READERS) {
            let (kept, dropped): (Vec<KeptPut>, Vec<KeptPut>) = std::mem::take(list).into_iter().partition(|p| !held.contains(&p.part) || p.rel == READERS);
            self.bytes -= dropped.iter().map(held_bytes).sum::<usize>();
            *list = kept;
            *at = list.iter().enumerate().map(|(i, p)| ((p.rel.clone(), p.ident, p.part), i)).collect();
        }
        for p in puts {
            self.bytes += held_bytes(&p);
            let key = (p.rel.clone(), p.ident, p.part);
            match at.get(&key) {
                Some(&i) => {
                    let was = &mut list[i];
                    if p.rel == READERS {
                        // A reader record only grows (decision 0401 step 8): two of a wave are one, naming both's.
                        let all: BTreeSet<[u8; 32]> = put_idents(was).into_iter().chain(put_idents(&p)).collect();
                        *was = readers_put(p.ident, &all);
                    } else if matches!(p.what, KeptWhat::Reads) && !matches!(was.what, KeptWhat::Reads | KeptWhat::Gone) {
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
        let keys = spaces.iter().map(|(s, _)| (space_key(s), None)).collect();
        // Its rows staged now (`KeptStage`); the rest, and the links, at its wave's commit.
        let (mut staged, mut rest, mut staged_bytes) = (Vec::new(), Vec::new(), 0);
        for (space, puts) in spaces {
            let (now, later): (Vec<KeptPut>, Vec<KeptPut>) = puts.into_iter().partition(stages);
            staged_bytes += now.iter().map(held_bytes).sum::<usize>();
            if !now.is_empty() {
                staged.push((space, now));
            }
            if !later.is_empty() {
                rest.push((space, later));
            }
        }
        let slot = (!staged.is_empty()).then(StagedSlot::default);
        let now = slot.clone().map(|slot| (staged_bytes, super::write_back::Work(super::write_back::Kind::KeptStage(Box::new(KeptStage { spaces: staged, slot })))));
        Some(super::write_back::Closed {
            what: "kept facts",
            bytes: self.bytes - staged_bytes,
            keys,
            syms: super::write_back::Syms::Mapped,
            job: super::write_back::Work(super::write_back::Kind::Kept(Box::new(KeptWrite { spaces: rest, staged: slot }))),
            now,
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
        // The run's program reads what it puts: its reader record goes in the same file (decision 0401 step 8), so no
        // collection sees the entries without it.
        let mut puts = puts;
        if puts.iter().any(|p| p.rel != READERS)
            && let Some(record) = rerail_prof::code_rule_dsl::logic::profile::with_bound::<KeptReader, _>(|r| {
                r.0.as_ref().and_then(|log| log.put(&space, puts.iter().filter(|p| p.rel != READERS && !matches!(p.what, KeptWhat::Gone)).map(|p| p.ident)))
            })
        {
            puts.push(record);
        }
        let mut puts = Some(puts);
        if self.gather("kept facts", KeptGather::default, |g| g.add(space, puts.take().expect("the puts"))) {
            return;
        }
        // Outside a run's waves (or with no writer): its own job, its puts folded as a wave's are (a later put of a key
        // replaces an earlier one, `KeptGather::add`), as the writer refuses a key put twice.
        let puts = KeptGather::folded(space, puts.take().expect("the puts"));
        let held: usize = puts.iter().map(held_bytes).sum();
        let write = KeptWrite { spaces: vec![(space, puts)], staged: None };
        self.write_back_syms(super::write_back::Syms::Mapped, "kept facts", held, vec![(space_key(&space), None)], super::write_back::Work(super::write_back::Kind::Kept(Box::new(write))));
    }

    /// [`Self::queue_kept`] as a job of its own called `what`: a write a warm run may make
    /// (`profile::WARM_WRITES_ALLOWED`, the journal's aged cursor) is so named, not gathered with the run's others.
    pub fn queue_kept_as(&self, space: [u8; 32], puts: Vec<KeptPut>, what: &'static str) {
        if what == "kept facts" {
            return self.queue_kept(space, puts);
        }
        if puts.is_empty() {
            return;
        }
        let puts = KeptGather::folded(space, puts);
        let held: usize = puts.iter().map(held_bytes).sum();
        let write = KeptWrite { spaces: vec![(space, puts)], staged: None };
        self.write_back_syms(super::write_back::Syms::Mapped, what, held, vec![(space_key(&space), None)], super::write_back::Work(super::write_back::Kind::Kept(Box::new(write))));
    }

    /// Waits until every kept write this process queued for `space` is written, reading nothing: so it may be asked
    /// as a thread ends, its connection gone.
    pub fn settle_kept(&self, space: &[u8; 32]) {
        self.settle(&space_key(space));
    }

    /// The rows of `space`'s chain once every kept write this process queued for it is written (`settle`): what a
    /// run reads before its first wave (decision 0104), its own earlier runs' writes included. `None` when it keeps
    /// none.
    pub fn kept_row(&self, space: &[u8; 32]) -> StoreResult<Option<ChainRows>> {
        self.settle(&space_key(space));
        // Once the snapshot holds the store's rows (read before the first wave, kept current by this process's
        // writer), a space it holds none of keeps none: answered with no SQL inside the waves (decision 0104).
        if let Some(Some(held)) = super::held::snapshot_kept(&self.database_path, |kept| kept.as_ref().map(|rows| rows.get(space).cloned())) {
            return Ok(held.filter(|r| !r.is_empty()));
        }
        let connection = self.connection.lock().expect("content store mutex poisoned");
        Ok(Some(chain_rows(&connection, space)?).filter(|r| !r.is_empty()))
    }

    /// Every space's row, read once a process before a run's first wave (decision 0104): from then on `kept_row`
    /// answers from memory, and this process's writer keeps it current. Returns the spaces read now (none when an
    /// earlier run of the process read them).
    pub fn preload_kept(&self) -> StoreResult<usize> {
        // Held under the snapshot's lock while read: this process's writer, committing meanwhile, replaces its row
        // after this read, not before it.
        super::held::snapshot_kept(&self.database_path, |kept| self.read_kept_rows(kept)).unwrap_or(Ok(0))
    }

    fn read_kept_rows(&self, kept: &mut Option<HashMap<[u8; 32], ChainRows>>) -> StoreResult<usize> {
        if kept.is_some() {
            return Ok(0);
        }
        let mut rows: HashMap<[u8; 32], ChainRows> = HashMap::new();
        {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut st = connection.prepare("SELECT space, seq, name, tag FROM main.facts_chain ORDER BY space, seq")?;
            let read = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)? as u64)))?;
            for r in read {
                let (space, seq, name, tag) = r?;
                if let Ok(space) = <[u8; 32]>::try_from(space.as_slice()) {
                    rows.entry(space).or_default().push((seq, name, tag));
                }
            }
        }
        let n = rows.len();
        *kept = Some(rows);
        Ok(n)
    }

    /// Collection's roll-up of the kept spaces (decision 0401 F4): each space whose reader records name a dead
    /// program (a plan the keep decision marked dead, `corpus_plans.live = 0`) written again as one base without the
    /// entries only dead programs' records name, nor those records; a space left with none is dropped. An entry no
    /// record names (an older build's, a session's) stays; so does one a live program's record names, and one of a
    /// program the store has not decided on. Each partition whose holder (`HOLDER`) the store no longer keeps, or whose
    /// plan is dead, goes too, with its holder. Run under the collection's lease, before the dead plans' rows go. The
    /// spaces rewritten or dropped.
    pub fn collect_kept_facts(&self) -> StoreResult<usize> {
        let (dead, spaces): (HashSet<[u8; 32]>, Vec<[u8; 32]>) = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let dead = connection.prepare("SELECT plan_digest FROM corpus_plans WHERE live=0")?.query_map([], |r| r.get::<_, Vec<u8>>(0))?.filter_map(|p| p.ok().and_then(|p| <[u8; 32]>::try_from(p.as_slice()).ok())).collect();
            let spaces = connection.prepare("SELECT DISTINCT space FROM facts_chain")?.query_map([], |r| r.get::<_, Vec<u8>>(0))?.filter_map(|s| s.ok().and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok())).collect();
            (dead, spaces)
        };
        let path = self.database_path.clone();
        let mut done = 0;
        for space in spaces {
            // A space with neither holders nor, while plans are dead, reader records has nothing to collect: no write
            // transaction for it.
            if let Some(kept) = self.kept_space(&space)? {
                let chain = kept.view();
                if chain.of(HOLDER).next().is_none() && (dead.is_empty() || chain.of(READERS).next().is_none()) {
                    continue;
                }
            }
            let written = self.immediate_transaction(|t| {
                let rows = chain_rows(t, &space)?;
                let old = KeptSpace::open(&path, &space, &rows)?;
                let chain = old.view();
                // The partitions whose holder the store no longer keeps (`HOLDER`).
                let mut unheld = HashSet::new();
                for entry in chain.of(HOLDER) {
                    let held = match holder_of(&entry) {
                        Some((_, Some(plan))) if dead.contains(&plan) => false,
                        Some((key, _)) => super::objects::source_id_by_hash(t, super::HashKey::from_sha256_digest(key))?.is_some(),
                        None => false,
                    };
                    if !held {
                        unheld.insert(entry.key.part);
                    }
                }
                let (mut live, mut gone) = (HashSet::new(), HashSet::new());
                let mut any_dead = false;
                if !dead.is_empty() {
                    for record in chain.of(READERS) {
                        let is_dead = dead.contains(record.key.ident);
                        any_dead |= is_dead;
                        (if is_dead { &mut gone } else { &mut live }).extend(record_idents(&record));
                    }
                }
                if !any_dead && unheld.is_empty() {
                    return Ok(None);
                }
                let keep = |k: &FactKey<'_>| {
                    if k.rel == READERS {
                        return !dead.contains(k.ident);
                    }
                    !unheld.contains(&k.part) && (live.contains(k.ident) || !gone.contains(k.ident))
                };
                rewrite_space(&path, t, &space, &keep)
            })?;
            let Some(rows) = written else { continue };
            done += 1;
            if rows.is_empty() {
                drop_space_files(&path, &space);
            } else {
                sweep_space(&path, &space, &rows);
            }
            super::held::snapshot_kept(&path, |kept| {
                if let Some(kept) = kept {
                    kept.insert(space, rows);
                }
            });
        }
        Ok(done)
    }

    /// `space`'s chain as `rows` (`kept_row`'s) name it, mapped, with the rows it was opened by; `None` when it keeps
    /// none. Where a file `rows` name is gone, they are rows a writer has since replaced: the rows this process holds
    /// are kept current by its own writer alone, and another writes the store too (another shared object's writer in
    /// this process, or another process), whose next write sweeps the files its commit replaced. The store's last
    /// commit is then read, held in their place, and opened. A file its rows name that is gone is corruption.
    pub fn open_kept(&self, space: &[u8; 32], rows: ChainRows) -> StoreResult<Option<(ChainRows, KeptSpace)>> {
        match KeptSpace::mapped(&self.database_path, space, &rows) {
            Ok(kept) => return Ok(Some((rows, kept))),
            Err((path, e)) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(StoreError::Corrupt(format!("a space's kept facts: {}: {e}", path.display())));
            }
            Err(_) => {}
        }
        let now = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            chain_rows(&connection, space)?
        };
        super::held::snapshot_kept(&self.database_path, |kept| {
            if let Some(kept) = kept {
                kept.insert(*space, now.clone());
            }
        });
        if now.is_empty() {
            return Ok(None);
        }
        let kept = KeptSpace::open(&self.database_path, space, &now)?;
        Ok(Some((now, kept)))
    }

    /// The kept facts of `space` as this process holds its chain's rows (else the store's last commit), mapped
    /// (`open_kept`); `None` when it keeps none.
    pub fn kept_space(&self, space: &[u8; 32]) -> StoreResult<Option<KeptSpace>> {
        let Some(rows) = self.kept_row(space)? else { return Ok(None) };
        Ok(self.open_kept(space, rows)?.map(|(_, kept)| kept))
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

/// A relation and part kind's entries, rows and bytes (`by_relation`).
#[cfg(any(test, feature = "test-hooks"))]
pub type KeptOf = ((String, &'static str), (usize, usize, usize));

/// What the store `path`'s chain keeps, by relation and part kind ("rows" for part 0, "granule" for an input's
/// granules, part > 0, "beside" for the parts beside them, part < 0): entries, rows and bytes, largest first.
#[cfg(any(test, feature = "test-hooks"))]
pub fn by_relation(path: &std::path::Path) -> StoreResult<Vec<KeptOf>> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let spaces: Vec<Vec<u8>> = c.prepare("SELECT DISTINCT space FROM facts_chain").and_then(|mut q| q.query_map([], |r| r.get(0))?.collect()).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let mut by: std::collections::HashMap<(String, &'static str), (usize, usize, usize)> = Default::default();
    for s in &spaces {
        let space: [u8; 32] = s.as_slice().try_into().map_err(|_| StoreError::Invalid("a space of another length".into()))?;
        let kept = KeptSpace::open(path, &space, &chain_rows(&c, &space)?)?;
        for k in kept.view().all() {
            let bytes = (0..k.rows.arity as usize).map(|col| k.rows.col(col).bytes()).sum::<usize>() + k.rows.levels().map_or(0, |l| l.bytes());
            let what = match k.key.part { 0 => "rows", p if p > 0 => "granule", _ => "beside" };
            let e = by.entry((k.key.rel.to_owned(), what)).or_default();
            (e.0, e.1, e.2) = (e.0 + 1, e.1 + k.rows.len(), e.2 + bytes);
        }
    }
    let mut rows: Vec<_> = by.into_iter().collect();
    rows.sort_by_key(|(_, (_, _, b))| std::cmp::Reverse(*b));
    Ok(rows)
}


#[cfg(test)]
mod tests;
