//! The tape values a chunk's run admits inside its fixpoint (decision 0431 §8).
//!
//! A string a rule parses is answered inside the fixpoint (`rr_embed_req`'s answer, decision 0065 §2a): its tape is
//! found by its text's sha or parsed, and admitted once a chunk. Admitted, it is projected alone into the chunk's
//! block (`Grower::admit`), and each relation and view the batch makes has the value's rows as a part of its own.
//! The chunk's sources are bound as [`Layered`]: the batch's source (its base) and the parts admitted since, whose
//! rows are numbered past the base's. Every read of a base row is the base's; a lookup is the base's and those of the
//! parts that hold the looked-up node's key (a value's own, or a host's its places name); a scan reads the base and
//! then the parts admitted so far.
//!
//! A value's node is reached through a guest relation, which binds it (0058's hiding), so a rule looks it up after the
//! answer that admitted it: its rows are there when they are read. A read the base answers from an index of its own
//! rows (`sorted_rows`, `own_index`, a keyed read) cannot hold a part's, so a layered source offers none of those.
//!
//! The device reads them where they are, as it reads the batch's (decision 0353). A value's relation rows are written
//! once past the chunk's, in the same block relation (`Grower::admitted`, a part a [`Slice`] of it), which the device
//! binds whole (`in_block`). A view's rows are read in place through one directory of the batch's documents and the
//! values' (`device_chunk::views_over`), made again when values are admitted after it.

use rerail_blocks::vector::Vector;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use super::super::DeviceView;
use super::super::Source;
use super::PythonBatch as _;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// The end of a key's chain (`Shard::chained`).
const END: u32 = u32::MAX;

/// One admitted value's rows of a relation: its source (a batch of the value alone), from row `start` of the parts.
#[derive(Clone)]
struct Part {
    start: usize,
    src: Arc<dyn Source>,
}

/// Parts in blocks of `BLOCK`, each block shared by the snapshots that hold it: a snapshot made from the last
/// (`Values::add`) copies the last block alone, not every part before it.
const BLOCK: usize = 64;

#[derive(Clone, Default)]
struct Blocks {
    blocks: Vec<Arc<Vec<Part>>>,
    len: usize,
}

impl Blocks {
    fn len(&self) -> usize {
        self.len
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn iter(&self) -> impl Iterator<Item = &Part> {
        self.blocks.iter().flat_map(|b| b.iter())
    }
    fn push(&mut self, part: Part) {
        if self.len.is_multiple_of(BLOCK) {
            self.blocks.push(Arc::new(Vec::with_capacity(BLOCK)));
        }
        Arc::make_mut(self.blocks.last_mut().expect("a block")).push(part);
        self.len += 1;
    }
    /// The first part for which `pred` is false, `pred` true of every part before it and false of every one after.
    fn partition_point(&self, pred: impl Fn(&Part) -> bool) -> usize {
        let b = self.blocks.partition_point(|b| b.last().is_some_and(&pred));
        self.blocks.get(b).map_or(self.len, |block| b * BLOCK + block.partition_point(&pred))
    }
}

impl<'a> IntoIterator for &'a Blocks {
    type Item = &'a Part;
    type IntoIter = std::iter::FlatMap<std::slice::Iter<'a, Arc<Vec<Part>>>, std::slice::Iter<'a, Part>, fn(&'a Arc<Vec<Part>>) -> std::slice::Iter<'a, Part>>;
    fn into_iter(self) -> Self::IntoIter {
        let parts: fn(&'a Arc<Vec<Part>>) -> std::slice::Iter<'a, Part> = |b| b.iter();
        self.blocks.iter().flat_map(parts)
    }
}

impl std::ops::Index<usize> for Blocks {
    type Output = Part;
    fn index(&self, i: usize) -> &Part {
        &self.blocks[i / BLOCK][i % BLOCK]
    }
}

/// The parts that hold each key's rows, in `SHARDS` maps by key, each shared by the snapshots that hold it: a snapshot
/// made from the last copies the shards of the keys it adds alone.
const SHARDS: usize = 256;

#[derive(Clone)]
struct Keys<K: ShardKey = u32> {
    shards: Vec<Arc<Shard<K>>>,
}

/// One shard of the keys, in blocks (decision 0353): per key, the first and last links of its chain in `chained`;
/// each link a part and the next link of its key's chain (`END` at the last).
#[derive(Clone, Default)]
struct Shard<K: ShardKey> {
    by_key: rerail_blocks::maps::BlockHash<K, [u32; 2]>,
    chained: Vector<[u32; 2]>,
}

impl<K: ShardKey> Default for Keys<K> {
    fn default() -> Self {
        Keys { shards: Vec::new() }
    }
}

/// A key's shard: its bits mixed, so that keys that differ in their low bits alone (a document's nodes) spread.
trait ShardKey: rerail_blocks::maps::Key + Default {
    fn shard(self) -> usize;
}
impl ShardKey for u32 {
    fn shard(self) -> usize {
        (self.wrapping_mul(0x9E37_79B9) >> 24) as usize % SHARDS
    }
}
impl ShardKey for i64 {
    fn shard(self) -> usize {
        ((self as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as usize % SHARDS
    }
}

impl<K: ShardKey> Keys<K> {
    fn shard(k: K) -> usize {
        k.shard()
    }
    /// The parts that hold rows of key `k`, in the order admitted.
    fn parts(&self, k: K) -> impl Iterator<Item = usize> + '_ {
        let shard = self.shards.get(Self::shard(k));
        let mut at = shard.and_then(|s| s.by_key.get(&k)).map_or(END, |c| c[0]);
        std::iter::from_fn(move || {
            let [part, next] = *shard?.chained.as_slice().get(at as usize)?;
            at = next;
            Some(part as usize)
        })
    }
    /// Part `i` holds rows of key `k`.
    fn add(&mut self, k: K, i: usize) {
        if self.shards.is_empty() {
            self.shards = (0..SHARDS).map(|_| Arc::default()).collect();
        }
        let shard = &self.shards[Self::shard(k)];
        if shard.by_key.get(&k).is_some_and(|c| shard.chained[c[1] as usize][0] == i as u32) {
            return;
        }
        let shard = Arc::make_mut(&mut self.shards[Self::shard(k)]);
        let link = shard.chained.len() as u32;
        match shard.by_key.get_mut(&k) {
            Some(chain) => {
                let last = std::mem::replace(&mut chain[1], link);
                shard.chained[last as usize][1] = link;
            }
            None => {
                shard.by_key.insert(k, [link, link]);
            }
        }
        shard.chained.push([i as u32, END]);
    }
}

/// A relation's parts, in the order admitted, each found by the keys of the nodes it holds rows of. The index of the
/// parts' sources, not their rows: an add makes the next one from it (`Values::add`), sharing what it does not change.
#[derive(Clone, Default)]
struct Parts {
    list: Blocks,
    total: usize,
    /// The parts by the document keys of the node ids their rows hold (`Keys`).
    by_key: Keys,
    /// The parts by the keys of their documents and those documents' files: a lookup by a document key (`Parts::of`).
    by_doc: Keys,
    /// The parts that hold each node of a document not theirs (a host's node their rows name: `guest_of`'s string,
    /// `host_node`'s host), by the node itself. Keyed by its document instead, a probe of any node of a file with
    /// parsed strings asked every part of them (t50: `text` and `!guest_of` at 1.5–2.5 µs a probe).
    hosted: Keys<i64>,
    /// The chunk's relation with every part's rows past its own, in one block relation, while every part is a
    /// [`Slice`] of it, each past the one before.
    whole: Option<Arc<super::Relation>>,
    sliced: bool,
}

impl Parts {
    /// The part holding row `row` of the parts.
    fn at(&self, row: usize) -> Option<(&Part, usize)> {
        let i = self.list.partition_point(|p| p.start <= row).checked_sub(1)?;
        let p = &self.list[i];
        (row < p.start + p.src.len()).then(|| (p, row - p.start))
    }
    /// The parts that may hold rows whose value is `v`: those that hold node ids of `v`'s key, and those whose rows
    /// name node `v` of a document not theirs (`hosted`); where the column's values are document keys (`docs`), those
    /// of the document `v` (a part is found by its documents' keys and their files', `Values::add`): no part is read
    /// for a key it does not hold. Every part for a value no key names (an atom, a number).
    fn of(&self, v: i64, docs: bool, each: &mut dyn FnMut(&Part)) {
        let key = if docs { u32::try_from(v).ok() } else { (1 << 32..).contains(&v).then_some((v >> 32) as u32) };
        let Some(key) = key else {
            if !docs {
                self.list.iter().for_each(each);
            }
            return;
        };
        for part in if docs { &self.by_doc } else { &self.by_key }.parts(key) {
            each(&self.list[part]);
        }
        if !docs {
            for part in self.hosted.parts(v) {
                each(&self.list[part]);
            }
        }
    }
}

/// The chunk's admitted values: each relation's parts, shared by its layered sources.
#[derive(Default)]
pub struct Values {
    /// Each relation's parts, as the values admitted so far make them: read without a lock, as every join's probe of a
    /// layered source reads them, and replaced whole by `add`. Read under a read-write lock, the probes of a chunk's
    /// jobs wrote its count of readers each time: a third of the running threads' time spun on it (R7, 0494 step 5).
    parts: arc_swap::ArcSwap<FxHashMap<String, Arc<Parts>>>,
    /// Held while `add` makes the next `parts` from the last.
    adding: parking_lot::Mutex<()>,
    admitted: AtomicUsize,
    /// Each admitted value's batch, in the order admitted: the documents a view's parts are rows of.
    batches: RwLock<Vec<Arc<super::Batch>>>,
    /// The views' directory over the chunk's batch and the values' (`device_chunk::views_over`), and the values it
    /// was made with.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    dir: parking_lot::Mutex<Option<(usize, Option<Arc<Dir>>)>>,
    /// Per relation made on lookup (an axis), the device's solve of the chunk's batch and its values in one relation
    /// (`Within::solved_with`), and the parts it was made with.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    solved: parking_lot::Mutex<FxHashMap<String, (usize, Option<crate::code_rule_dsl::facts::rows::Part>)>>,
}

/// A views' directory of a chunk's batch and its values, with what its rows' entries and tapes are kept by.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub(super) struct Dir {
    pub(super) views: super::device_chunk::ViewsInPlace,
    batches: Vec<Arc<super::Batch>>,
}

impl Values {
    /// Values `own`'s sources: each relation's rows of them, a part of that relation from now on. `own` are the
    /// values' keys; `hosts` the keys of the documents their places are in, whose nodes their rows name.
    /// A part is found by its own keys, and by each node of another document its rows name (`hosted`), read off its
    /// rows once here. A relation made on lookup has no rows yet: it is found by its hosts' keys too.
    /// The next `parts` is made from the last: a relation the value adds rows to is made again from its last (its
    /// last block of parts and the shards of the keys it adds copied, `Blocks`, `Keys`), the others shared.
    pub fn add(&self, own: &[u32], hosts: &[u32], sources: Vec<(String, Arc<dyn Source>)>) {
        let _adding = self.adding.lock();
        let mut parts: FxHashMap<String, Arc<Parts>> = (**self.parts.load()).clone();
        // The keys a part is found by in a lookup by document (`Parts::of`): the values' and their hosts', and its
        // documents' and their files'.
        // A relation whose rows are its documents' own (`Source::rows_of_own_documents`, a view of the value's tape) is
        // found by those documents' keys alone: a probe of a host's node reads none of its values' views.
        let (mut docs, mut own_docs): (Vec<u32>, Vec<u32>) = (own.iter().chain(hosts).copied().collect(), Vec::new());
        if let Some(b) = sources.iter().find_map(|(_, s)| s.python_batch()) {
            for d in 0..b.docs.len() {
                docs.extend([b.docs.key(d), b.file(d)]);
                own_docs.push(b.docs.key(d));
            }
            own_docs.sort_unstable();
            own_docs.dedup();
            self.batches.write().push(b);
        }
        docs.sort_unstable();
        docs.dedup();
        for (name, src) in sources {
            // A relation made on lookup (containment, an axis) has its rows only once a lookup asks. The long-text
            // nodes' source holds the value's batch, which every node read of the value goes through
            // (`LongTexts::of`): kept with no rows too, so a value's node is read in its own batch (a string's value in a
            // Vue piece, decision 0597).
            if src.len() == 0 && !src.made_on_lookup() && name != super::LONG_TEXT_NODES {
                continue;
            }
            // The keys its nodes are found by: the long-text nodes' the value's own alone (a host file's node read
            // under its places' hosts' keys too, `value` of a `getattr`'s string, went to a string annotation's batch,
            // which holds no node of the file, and found nothing); a view of the value's own documents' those
            // documents'; any other the value's own, and the hosts' nodes its rows name (`hosted`). `own` is every
            // value of the round (`admit_many`), the hosts apart (`hosts`): keyed by the first alone, a round's later
            // values' nodes were read in no batch (`Optional` of R10's second annotation).
            let (keyed, general) = if name == super::LONG_TEXT_NODES {
                (own, false)
            } else if src.rows_of_own_documents() && !own_docs.is_empty() {
                (&own_docs[..], false)
            } else {
                (own, true)
            };
            let p = Arc::make_mut(parts.entry(name).or_default());
            let i = p.list.len();
            // Its rows past the chunk's and the parts' before, in their block relation: the device binds that whole.
            let slice = src.as_any().and_then(|a| a.downcast_ref::<Slice>());
            p.sliced = (i == 0 || p.sliced) && slice.is_some_and(|sl| p.whole.as_ref().is_none_or(|w| w.len() == sl.from));
            p.whole = slice.filter(|_| p.sliced).map(|sl| Arc::clone(&sl.whole));
            p.list.push(Part { start: p.total, src: Arc::clone(&src) });
            p.total += src.len();
            for &k in &docs {
                p.by_doc.add(k, i);
            }
            for &k in keyed {
                p.by_key.add(k, i);
            }
            if !general {
                continue;
            }
            if src.made_on_lookup() {
                // No rows yet: found by its hosts' keys too.
                for &k in hosts {
                    p.by_key.add(k, i);
                }
            } else {
                // Each node of another document its rows name, read off its rows once, by the source's walk: a view
                // read in place answers its `len` from its statistics before its rows are walked, and is not read a
                // row at a time.
                let columns: Vec<usize> = (0..src.columns().len()).collect();
                let mut named: rustc_hash::FxHashSet<i64> = rustc_hash::FxHashSet::default();
                src.for_rows(&columns, &mut |_, row| {
                    named.extend(row.iter().copied().filter(|&v| (1 << 32..).contains(&v) && !own.contains(&((v >> 32) as u32))));
                });
                for v in named {
                    p.hosted.add(v, i);
                }
            }
        }
        self.parts.store(Arc::new(parts));
        // After the parts: a reader that counts a value admitted finds its parts (`has_parts`,
        // `Layered::lookup_read_at`).
        self.admitted.fetch_add(1, Ordering::Release);
    }
    /// The values admitted.
    pub fn admitted(&self) -> usize {
        self.admitted.load(Ordering::Acquire)
    }
    /// The source of relation `name` that holds rows of node `n`'s document: an admitted value's.
    pub fn source_of(&self, name: &str, n: i64) -> Option<Arc<dyn Source>> {
        let parts = self.parts.load();
        let p = parts.get(name)?;
        let i = p.by_key.parts((n >> 32) as u32).next()?;
        Some(Arc::clone(&p.list[i].src))
    }
    fn total(&self, name: &str) -> usize {
        self.parts.load().get(name).map_or(0, |p| p.total)
    }
    /// The chunk's relation `name` with its values' rows, in one block relation, when every part is in it.
    fn whole(&self, name: &str) -> Option<Arc<super::Relation>> {
        self.parts.load().get(name).and_then(|p| p.whole.clone())
    }
    /// The views' directory of `batch` (the chunk's) and the values admitted so far: made again when more are.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn dir(&self, batch: &super::Batch) -> Option<Arc<Dir>> {
        let batches = self.batches.read().clone();
        let mut dir = self.dir.lock();
        if let Some((n, made)) = dir.as_ref()
            && *n == batches.len()
        {
            return made.clone();
        }
        let made = super::device_chunk::views_over(batch, &batches).map(|views| Arc::new(Dir { views, batches: batches.clone() }));
        *dir = Some((batches.len(), made.clone()));
        made
    }
}

/// Rows `from..` of a relation that holds a chunk's rows and then its values' (`Grower::admitted`): one value's rows of
/// it, numbered from 0, read where they were written.
pub struct Slice {
    whole: Arc<super::Relation>,
    from: usize,
    len: usize,
}

impl Slice {
    pub(super) fn new(whole: Arc<super::Relation>, from: usize) -> Self {
        let len = whole.len() - from;
        Slice { whole, from, len }
    }
}

impl Source for Slice {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        self.whole.columns()
    }
    fn len(&self) -> usize {
        self.len
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        1
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    // A value's rows are few: read whole (`Layered::lookup`).
    fn lookup(&self, _: usize, _: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.whole.value(self.from + row, column)
    }
}

/// Where a layered source made on lookup numbers the rows it makes of a value's (`Made`): past every row containment
/// makes (`within`'s outer rows start at `1 << 31`, and a chunk makes far fewer than `1 << 30` of them), within the
/// `u32` the engine reads a row by.
const MADE_AT: usize = 0xF000_0000;
const MADE_ROOM: usize = (1 << 32) - MADE_AT;

/// The rows a layered source made on lookup made of the values' parts (containment's, an axis's answers): each
/// looked-up value's rows written once, where they are computed, as `within` writes its own.
#[derive(Default)]
struct Made {
    rows: Vector<i64>,
    /// Each looked-up value's run of rows, and how many parts there were when it was made: a part admitted since (a
    /// value given a place more) makes it again.
    /// Under (column, value): [first row, rows, parts].
    runs: rerail_blocks::maps::BlockHash<[i64; 2], [u64; 3]>,
    /// The values' rows as a scan reads them (`Layered::scanned`), once one has.
    scan: Option<Arc<Scan>>,
}

/// A layered source made on lookup, scanned: its values' rows past the base's, which the scan made whole, so a scan
/// reads the rows a lookup does (decision 0597: a rule over a Vue template's pieces, `vcode(_, g, _), has(g, r, S)`,
/// planned the axis's scan first, and the scan held none of the pieces' rows). Every part's rows are made whole once,
/// at the first scan; the parts admitted since add a segment past them, each written once where it is made (decision
/// 0353), a reader holding the segments it read. A lookup then reads the same rows (`Seg::index`).
struct Scan {
    /// The first row: the base's count, every one of its rows made by its scan.
    at: usize,
    /// The parts made whole.
    parts: usize,
    segs: Vec<Arc<Seg>>,
}

/// The rows of some parts, made whole: from row `first` of the scan's.
struct Seg {
    first: usize,
    rows: Vector<i64>,
    /// Each row's (column, value, row), sorted: the rows that hold a value are one run of it (`holding`), read in
    /// place (decision 0628: a run's data in blocks).
    index: Vector<[i64; 3]>,
}

impl Scan {
    fn len(&self, width: usize) -> usize {
        self.segs.last().map_or(0, |s| s.first + s.rows.len() / width)
    }
    /// Whether row `row` of the source is the scan's: the base's rows made on lookup are past its count too
    /// (`within`'s at `1 << 30` and `1 << 31`).
    fn holds(&self, row: usize, width: usize) -> bool {
        (self.at..self.at + self.len(width)).contains(&row)
    }
    /// Row `i` of the scan's (from `at`).
    fn row(&self, i: usize, width: usize) -> &[i64] {
        let k = self.segs.partition_point(|s| s.first <= i) - 1;
        let s = &self.segs[k];
        let at = (i - s.first) * width;
        &s.rows[at..at + width]
    }
    /// Every row, by its number in the source.
    fn each(&self, width: usize, f: &mut dyn FnMut(usize, &[i64]) -> bool) {
        for s in &self.segs {
            for (i, r) in s.rows.chunks(width).enumerate() {
                if !f(self.at + s.first + i, r) {
                    return;
                }
            }
        }
    }
    /// The rows whose `column` is `v`, by their numbers in the source.
    fn holding(&self, column: usize, v: i64) -> impl Iterator<Item = usize> + '_ {
        let key = [column as i64, v];
        self.segs.iter().flat_map(move |s| {
            let lo = s.index.partition_point(|e| [e[0], e[1]] < key);
            let hi = lo + s.index[lo..].partition_point(|e| [e[0], e[1]] == key);
            s.index[lo..hi].iter().map(move |e| self.at + s.first + e[2] as usize)
        })
    }
}

/// A chunk's source of relation `name`: the batch's (`base`), and the values admitted since, past its rows.
pub struct Layered {
    name: String,
    base: Arc<dyn Source>,
    /// The batch's rows, counted when a value's rows are first numbered past them: a source the chunk binds is
    /// layered whether or not a rule reads it, and a count of one made on lookup (`within`) makes every row.
    base_len: std::sync::OnceLock<usize>,
    values: Arc<Values>,
    /// For a base made on lookup, whose rows grow as it is read: the parts' rows, made as they are looked up.
    made: Option<parking_lot::Mutex<Made>>,
    width: usize,
    /// `has_parts`' last answer and the values admitted then (`admitted << 1 | has`): asked again only when more are.
    seen: AtomicU64,
}

impl Layered {
    pub fn new(name: &str, base: Arc<dyn Source>, values: Arc<Values>) -> Self {
        let made = base.made_on_lookup().then(|| parking_lot::Mutex::new(Made::default()));
        let width = base.columns().len();
        Layered { name: name.to_owned(), base, base_len: std::sync::OnceLock::new(), values, made, width, seen: AtomicU64::new(0) }
    }
    /// The batch's rows, where a value's rows are numbered past them (never for a base made on lookup, whose values'
    /// rows are at `MADE_AT`).
    fn base_len(&self) -> usize {
        *self.base_len.get_or_init(|| self.base.len())
    }
    /// The batch's source.
    pub fn base(&self) -> &Arc<dyn Source> {
        &self.base
    }
    /// The source that holds node `n`'s document's rows: an admitted value's, else the batch's.
    pub fn source_of(&self, n: i64) -> Arc<dyn Source> {
        self.values.source_of(&self.name, n).unwrap_or_else(|| Arc::clone(&self.base))
    }
    /// The sources that may hold node `n`'s document: the chunk's batch's, then each admitted value's that names its
    /// key (a value's rows are keyed by its places' hosts too, so a host file's key names its guests' parts).
    pub fn sources_of(&self, n: i64) -> Vec<Arc<dyn Source>> {
        let mut out = vec![Arc::clone(&self.base)];
        if let Some(p) = self.values.parts.load().get(&self.name) {
            out.extend(p.by_key.parts((n >> 32) as u32).map(|i| Arc::clone(&p.list[i].src)));
        }
        out
    }
    fn parts<T>(&self, f: impl FnOnce(&Parts) -> T) -> Option<T> {
        if self.values.admitted.load(Ordering::Acquire) == 0 {
            return None;
        }
        self.values.parts.load().get(&self.name).filter(|p| !p.list.is_empty()).map(|p| f(p))
    }
    /// With no value admitted, no relation has parts: a probe of a chunk with none (most of them) reads only the
    /// count, where the parts' load (`arc_swap`'s, a thread-local each) and the name's lookup took about 600 samples
    /// of a 15 s sample of `all`.
    /// Asked once per value admitted: each probe's load of the parts and hash of the name took about 1% of `all`'s
    /// per-file phase's samples.
    fn has_parts(&self) -> bool {
        let admitted = self.values.admitted() as u64;
        if admitted == 0 {
            return false;
        }
        let seen = self.seen.load(Ordering::Relaxed);
        if seen >> 1 == admitted {
            return seen & 1 != 0;
        }
        let has = self.values.total(&self.name) > 0;
        self.seen.store(admitted << 1 | u64::from(has), Ordering::Relaxed);
        has
    }
    /// Made on lookup, with values admitted: the scan's rows of the values (`Scan`), made up to the parts admitted
    /// now; `f` reads them.
    fn scanned<T>(&self, f: impl FnOnce(&Scan) -> T) -> Option<T> {
        let made = self.made.as_ref()?;
        if !self.has_parts() {
            return None;
        }
        let width = self.width.max(1);
        let mut made = made.lock();
        if made.scan.is_none() {
            made.scan = Some(Arc::new(Scan { at: self.base.len(), parts: 0, segs: Vec::new() }));
        }
        let scan = made.scan.as_ref().expect("made above");
        let more = self.parts(|p| {
            (scan.parts < p.list.len()).then(|| {
                let first = scan.len(width);
                let mut seg = Seg { first, rows: Vector::new(), index: Vector::new() };
                for part in (&p.list).into_iter().skip(scan.parts) {
                    for r in 0..part.src.len() {
                        let row = (seg.rows.len() / width) as u32;
                        for c in 0..self.width {
                            let x = part.src.value(r, c);
                            seg.rows.push(x);
                            seg.index.push([c as i64, x, i64::from(row)]);
                        }
                    }
                }
                seg.index.sort_unstable();
                (p.list.len(), seg)
            })
        });
        if let Some((parts, seg)) = more.flatten() {
            // The segments are shared, never their rows: the scan a reader holds keeps the segments it read.
            let mut next = Scan { at: scan.at, parts, segs: scan.segs.clone() };
            next.segs.push(Arc::new(seg));
            assert!(next.at + next.len(width) <= MADE_AT, "{}: more rows scanned of the tape values than their room", self.name);
            made.scan = Some(Arc::new(next));
        }
        // Read with the lock released: a reader may read this source again.
        let scan = Arc::clone(made.scan.as_ref().expect("made above"));
        drop(made);
        Some(f(&scan))
    }
    /// The scan's rows of the values, when a scan has made them: `f` reads them.
    fn if_scanned<T>(&self, f: impl FnOnce(&Scan) -> T) -> Option<T> {
        let scanned = self.made.as_ref()?.lock().scan.is_some();
        if scanned { self.scanned(f) } else { None }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Layered {
    /// A view's rows on the device: the batch's and its values', read in place through their directory. The base's
    /// view as the device reads it (its kind), its directory that of every document.
    fn view_over(&self) -> Option<DeviceView> {
        let batch = self.base.python_batch()?;
        let mut dv = self.base.device_view();
        let dv_ref = dv.as_mut()?;
        let v = &mut dv_ref.in_place;
        let dir = self.values.dir(&batch)?;
        let views = &dir.views;
        let loc = (super::device_chunk::VIEW_LOC_LINE..=super::device_chunk::VIEW_LOC_DOCOF).contains(&dv_ref.view);
        if (dv_ref.view == super::device_chunk::VIEW_LINE && !views.lines) || (loc && !views.locs) {
            return None;
        }
        v.dir = views.dir?;
        v.buffers = views.buffers.clone();
        v.caches = views.caches.clone();
        v.keep = Arc::clone(&dir) as Arc<dyn std::any::Any + Send + Sync>;
        v.failed = v.cache_failed();
        dv_ref.rows = self.len();
        dv
    }

    /// An axis's rows on the device (a base made on lookup): the device's solve of the batch and of its values', off
    /// their tapes, in one relation, made again when values are admitted after it. Its rows are more than the source
    /// numbers (a value's are made on lookup, `MADE_AT`): a reader that binds it reads every pair.
    fn solved_in_block(&self) -> Option<crate::code_rule_dsl::facts::rows::InBlock> {
        let base = self.base.as_any()?.downcast_ref::<super::within::Within>()?;
        let parts = self.values.parts.load();
        let p = parts.get(&self.name)?;
        let mut solved = self.values.solved.lock();
        if let Some((n, made)) = solved.get(&self.name)
            && *n == p.list.len()
        {
            return made.as_ref().map(|w| w.in_block(w.len()));
        }
        let more: Option<Vec<&super::within::Within>> = p.list.iter().map(|x| x.src.as_any().and_then(|a| a.downcast_ref::<super::within::Within>())).collect();
        // Over the layered directory of the batch and its values (`views_over`), read off their tapes: one value's
        // axis a batch of it, in the order admitted.
        // The axis's own batch: an axis is no node source (`python_batch`).
        let dir = self.values.dir(base.batch())?;
        let made = more.filter(|m| m.len() == dir.batches.len()).and_then(|m| base.solved_with(&m, &dir.views, Arc::clone(&dir) as Arc<dyn std::any::Any + Send + Sync>));
        solved.insert(self.name.clone(), (p.list.len(), made.clone()));
        made.map(|w| w.in_block(w.len()))
    }
}

impl Source for Layered {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.base.syntax_batch()
    }
    /// A chunk's layered binding (decision 0431 §8): a node is read through the batch that holds its document, the
    /// chunk's or a tape value's. A value's parts are keyed by their hosts' keys too (`sources_of`), so a host file's
    /// key names its guests' parts as well: the first of them that holds the node answers.
    fn node_read(&self, name: &str, n: i64, more: &[i64]) -> Option<String> {
        self.sources_of(n).into_iter().find_map(|s| s.node_read(name, n, more))
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        self.base.columns()
    }
    fn text_column(&self, column: usize) -> bool {
        self.base.text_column(column)
    }
    fn len(&self) -> usize {
        // Made on lookup, the base's rows grow as it is read; a scan's are every one of them, then the values' made
        // whole past them (`Scan`). A lookup's of a value not scanned are at `MADE_AT`.
        if self.made.is_some() {
            return self.scanned(|s| s.at + s.len(self.width.max(1))).unwrap_or_else(|| self.base.len());
        }
        self.base_len() + self.values.total(&self.name)
    }
    fn estimated_len(&self) -> usize {
        self.base.estimated_len()
    }
    fn may_hold_rows(&self) -> bool {
        self.base.may_hold_rows() || self.values.total(&self.name) > 0
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.base.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.base.lookup_cost(column)
    }
    fn scan_cost(&self) -> f64 {
        self.base.scan_cost()
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.base.statistics()
    }
    /// The base's counts: the values admitted past its rows are a few (a chunk's tape values), and a count read
    /// across the rows made the base's batch whole.
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vector<i64>> {
        self.base.value_counts(wanted)
    }
    fn indexed(&self, column: usize) -> bool {
        self.base.indexed(column)
    }
    fn made_on_lookup(&self) -> bool {
        self.base.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.base.complete_rows()
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.base.expect_lookups(column, keys)
    }
    fn failure(&self) -> Option<String> {
        self.base.failure()
    }
    fn doc_column(&self) -> Option<usize> {
        self.base.doc_column()
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let mut out = self.base.lookup(column, values)?;
        if let Some(rows) = self.if_scanned(|s| values.iter().flat_map(|&v| s.holding(column, v)).collect::<Vec<_>>()) {
            out.extend(rows);
            return Some(out);
        }
        let docs = self.base.keys_documents(column);
        if let Some(made) = &self.made {
            self.parts(|p| {
                let mut made = made.lock();
                for &v in values {
                    let run = match made.runs.get(&[column as i64, v]) {
                        Some(&[first, n, parts]) if parts as usize == p.list.len() => (first as usize, n as usize),
                        _ => {
                            let at = made.rows.len() / self.width.max(1);
                            p.of(v, docs, &mut |part| {
                                for r in part.src.lookup(column, &[v]).unwrap_or_default() {
                                    for c in 0..self.width {
                                        let x = part.src.value(r, c);
                                        made.rows.push(x);
                                    }
                                }
                            });
                            let run = (at, made.rows.len() / self.width.max(1) - at);
                            assert!(at + run.1 <= MADE_ROOM, "{}: more rows made of the tape values than its room", self.name);
                            made.runs.insert([column as i64, v], [run.0 as u64, run.1 as u64, p.list.len() as u64]);
                            run
                        }
                    };
                    out.extend((run.0..run.0 + run.1).map(|r| MADE_AT + r));
                }
            });
            return Some(out);
        }
        self.parts(|p| {
            for &v in values {
                p.of(v, docs, &mut |part| {
                    let at = self.base_len() + part.start;
                    match part.src.lookup(column, &[v]) {
                        Some(rows) => out.extend(rows.into_iter().map(|r| at + r)),
                        // A part that keeps no index of `column` is read whole: a value's rows are few.
                        None => out.extend((0..part.src.len()).filter(|&r| part.src.value(r, column) == v).map(|r| at + r)),
                    }
                });
            }
        });
        Some(out)
    }
    /// `lookup`'s rows by position: the base's, then the parts' that may hold `value` (a value's rows are few), at
    /// their place past the base's. A base made on lookup numbers its values' rows as they are made (`MADE_AT`):
    /// `lookup`'s rows.
    fn lookup_seq(&self, column: usize, value: i64) -> Option<crate::code_rule_dsl::facts::vtab::Seq<'_>> {
        use crate::code_rule_dsl::facts::vtab::Seq;
        if !self.has_parts() {
            return self.base.lookup_seq(column, value);
        }
        if self.made.is_some() {
            return self.lookup(column, &[value]).map(Seq::rows);
        }
        let base = self.base.lookup_seq(column, value)?;
        let mut more = rerail_blocks::vector::Vector::new();
        let docs = self.base.keys_documents(column);
        self.parts(|p| {
            p.of(value, docs, &mut |part| {
                let at = self.base_len() + part.start;
                match part.src.lookup(column, &[value]) {
                    Some(rows) => more.extend(rows.into_iter().map(|r| at + r)),
                    // A part that keeps no index of `column` is read whole: a value's rows are few.
                    None => more.extend((0..part.src.len()).filter(|&r| part.src.value(r, column) == value).map(|r| at + r)),
                }
            });
        });
        Some(base.then(Seq::rows(more)))
    }
    /// The base's matches read as found, then each part's that may hold the value (`Parts::of`), numbered past the
    /// base's as `lookup` numbers them: a part with no such read is looked up and its rows read (a value's rows are
    /// few). Refused, a chunk with a value admitted read every probe's rows a point read each (`row_at`).
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.lookup_read_at(column, value, columns, &mut super::super::ProbeCursor::default(), each)
    }
    /// The cursor is the base's: a part's reads keep none (a value's rows are few).
    fn lookup_read_at(&self, column: usize, value: i64, columns: &[usize], cursor: &mut super::super::ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        // No value admitted (a chunk's first run): the base's read, with no load of the parts.
        if self.values.admitted.load(Ordering::Acquire) == 0 {
            return self.base.lookup_read_at(column, value, columns, cursor, each);
        }
        // The relation's parts, loaded and found once a probe (`has_parts`' and `parts`' one lookup by name).
        let all = self.values.parts.load();
        let Some(p) = all.get(&self.name).filter(|p| p.total > 0) else {
            return self.base.lookup_read_at(column, value, columns, cursor, each);
        };
        if self.made.is_some() || columns.len() > 8 || !self.base.lookup_read_at(column, value, columns, cursor, each) {
            return false;
        }
        if p.list.is_empty() {
            return true;
        }
        let (base, docs) = (self.base_len(), self.base.keys_documents(column));
        let mut row = [0i64; 8];
        let row = &mut row[..columns.len()];
        {
            p.of(value, docs, &mut |part| {
                let at = base + part.start;
                if part.src.lookup_read(column, value, columns, &mut |r, got| each(at + r, got)) {
                    return;
                }
                let rows = part.src.lookup(column, &[value]).unwrap_or_else(|| (0..part.src.len()).filter(|&r| part.src.value(r, column) == value).collect());
                for r in rows {
                    part.src.row_at(r, columns, row);
                    each(at + r, row);
                }
            })
        }
        true
    }
    fn keys_documents(&self, column: usize) -> bool {
        self.base.keys_documents(column)
    }
    fn rows_of_own_documents(&self) -> bool {
        self.base.rows_of_own_documents()
    }
    /// A range of the base's column (a node view's own), and of each part's: not made on lookup (a value's rows are
    /// then made per key, never all read).
    fn range_indexed(&self, column: usize) -> bool {
        self.made.is_none() && self.base.range_indexed(column)
    }
    /// The base's rows in the range, then each part's, past the base's: a part with no range read is read whole (a
    /// value's rows are few). `rr_ann_node`'s range of the annotations' nodes is the values' rows.
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        if self.made.is_some() {
            return None;
        }
        let mut out = self.base.range_lookup(column, lower, upper)?;
        let inside = |x: i64| lower.is_none_or(|(v, eq)| if eq { x >= v } else { x > v }) && upper.is_none_or(|(v, eq)| if eq { x <= v } else { x < v });
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len() + part.start;
                match part.src.range_indexed(column).then(|| part.src.range_lookup(column, lower, upper)).flatten() {
                    Some(rows) => out.extend(rows.into_iter().map(|r| at + r)),
                    None => out.extend((0..part.src.len()).filter(|&r| inside(part.src.value(r, column))).map(|r| at + r)),
                }
            }
        });
        Some(out)
    }
    /// The base's counts and each part's, a part with none counted by its rows (a value's rows are few).
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        if self.made.is_some() {
            return None;
        }
        let mut by: std::collections::BTreeMap<i64, u64> = self.base.column_counts(c)?.into_iter().collect();
        self.parts(|p| {
            for part in &p.list {
                match part.src.column_counts(c) {
                    Some(counts) => counts.into_iter().for_each(|(v, n)| *by.entry(v).or_default() += n),
                    None => (0..part.src.len()).for_each(|r| *by.entry(part.src.value(r, c)).or_default() += 1),
                }
            }
        });
        Some(by.into_iter().collect())
    }
    /// The base's order over several columns (`within`'s pair): without it a chunk's probe of both columns took the
    /// lead column's rows and filtered them, making every ancestor's row of each node it tested.
    fn composite(&self) -> bool {
        self.base.composite()
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.base.estimated_matches_all(columns)
    }
    /// The base's rows of the values; with values admitted, the lead column's rows (the base's and the parts', as
    /// `lookup` gives them) that hold every value.
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vector<usize>> {
        if !self.has_parts() {
            return self.base.lookup_all(equal);
        }
        let &(c, v) = equal.first()?;
        let rows = self.lookup(c, &[v])?;
        Some(rows.iter().copied().filter(|&r| equal.iter().all(|&(c, v)| self.value(r, c) == v)).collect())
    }
    /// The base's test (`within`'s interval, t50 step 4), else the test of each part that may hold the row's first
    /// value's node: a value's nodes are its part's, never the batch's. Unforwarded, a chunk's every `within` test was
    /// a lookup of the node's rows (`def_outside`: 1.56 M tests at 1.6 µs on 1,000 files).
    fn holds(&self, equal: &[(usize, i64)]) -> Option<bool> {
        let base = self.base.holds(equal)?;
        if base || !self.has_parts() {
            return Some(base);
        }
        let &(_, v) = equal.iter().min_by_key(|e| e.0)?;
        let mut held = Some(false);
        let docs = self.base.keys_documents(equal.iter().min_by_key(|e| e.0)?.0);
        self.parts(|p| {
            p.of(v, docs, &mut |part| {
                held = match (held, part.src.holds(equal)) {
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                };
            })
        });
        held
    }
    fn for_rows_keyed(&self, column: usize, keys: &rustc_hash::FxHashSet<i64>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        // The base's walk, then each part's rows of the keys (a value's rows are few: read whole where its source has
        // no walk). Made on lookup, the values' rows are made as looked up, never walked (`for_rows_in`).
        let mut going = true;
        if !self.base.for_rows_keyed(column, keys, columns, &mut |r, row| {
            going = each(r, row);
            going
        }) {
            return false;
        }
        if !going {
            return true;
        }
        if self.made.is_some() {
            self.scanned(|s| {
                let mut row = vec![0i64; columns.len()];
                s.each(self.width.max(1), &mut |i, r| {
                    if !keys.contains(&r[column]) {
                        return true;
                    }
                    for (o, &c) in row.iter_mut().zip(columns) {
                        *o = r[c];
                    }
                    each(i, &row)
                });
            });
            return true;
        }
        let mut row = vec![0i64; columns.len()];
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len() + part.start;
                let walked = going
                    && part.src.for_rows_keyed(column, keys, columns, &mut |r, row| {
                        going = each(at + r, row);
                        going
                    });
                if walked || !going {
                    continue;
                }
                for r in (0..part.src.len()).filter(|&r| keys.contains(&part.src.value(r, column))) {
                    part.src.row_at(r, columns, &mut row);
                    going = each(at + r, &row);
                    if !going {
                        break;
                    }
                }
            }
        });
        true
    }
    fn lookup_eq(&self, column: usize, value: i64, eqs: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        if self.has_parts() { None } else { self.base.lookup_eq(column, value, eqs) }
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        if self.has_parts() { None } else { self.base.lookup_in(column, values, doc) }
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.base.kept_to_doc(column)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(made) = &self.made {
            if row < MADE_AT {
                if let Some(x) = self.if_scanned(|s| s.holds(row, self.width.max(1)).then(|| s.row(row - s.at, self.width.max(1))[column])).flatten() {
                    return x;
                }
                return self.base.value(row, column);
            }
            return made.lock().rows.get((row - MADE_AT) * self.width + column).copied().unwrap_or(0);
        }
        if row < self.base_len() {
            return self.base.value(row, column);
        }
        self.parts(|p| p.at(row - self.base_len()).map(|(part, r)| part.src.value(r, column))).flatten().unwrap_or(0)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        if self.made.is_some() {
            if row < MADE_AT && !self.if_scanned(|s| s.holds(row, self.width.max(1))).unwrap_or(false) {
                return self.base.row_at(row, columns, out);
            }
            for (o, &c) in out.iter_mut().zip(columns) {
                *o = self.value(row, c);
            }
            return;
        }
        if row < self.base_len() {
            return self.base.row_at(row, columns, out);
        }
        self.parts(|p| {
            if let Some((part, r)) = p.at(row - self.base_len()) {
                part.src.row_at(r, columns, out);
            }
        });
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        self.base.for_rows(columns, each);
        if self.made.is_some() {
            self.scanned(|s| {
                let mut row = vec![0i64; columns.len()];
                s.each(self.width.max(1), &mut |i, r| {
                    for (o, &c) in row.iter_mut().zip(columns) {
                        *o = r[c];
                    }
                    each(i, &row);
                    true
                });
            });
            return;
        }
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len() + part.start;
                part.src.for_rows(columns, &mut |r, row| each(at + r, row));
            }
        });
    }
    fn for_rows_in(&self, range: Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        if self.made.is_some() {
            let Some(at) = self.scanned(|s| s.at) else { return self.base.for_rows_in(range, columns, each) };
            let mut going = true;
            if range.start < at {
                self.base.for_rows_in(range.start..range.end.min(at), columns, &mut |r, row| {
                    going = each(r, row);
                    going
                });
            }
            let mut row = vec![0i64; columns.len()];
            for r in range.start.max(at)..range.end.min(self.len()) {
                if !going {
                    return;
                }
                self.row_at(r, columns, &mut row);
                going = each(r, &row);
            }
            return;
        }
        let base = range.start.min(self.base_len())..range.end.min(self.base_len());
        let mut going = true;
        if !base.is_empty() {
            self.base.for_rows_in(base, columns, &mut |r, row| {
                going = each(r, row);
                going
            });
        }
        if !going || range.end <= self.base_len() {
            return;
        }
        let mut row = vec![0i64; columns.len()];
        for r in range.start.max(self.base_len())..range.end.min(self.len()) {
            self.row_at(r, columns, &mut row);
            if !each(r, &row) {
                return;
            }
        }
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.base.documents_of(column, each);
        if self.made.is_some() {
            // A value's node is its document's: the key its id holds.
            self.scanned(|s| {
                s.each(self.width.max(1), &mut |i, r| {
                    let id = r[column];
                    each(i, if id >= 1 << 32 { id >> 32 } else { id });
                    true
                });
            });
            return;
        }
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len() + part.start;
                part.src.documents_of(column, &mut |r, d| each(at + r, d));
            }
        });
    }
    // A relation's rows on the device: the chunk's, or its and its values' in their one block relation.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn in_block(&self) -> Option<crate::code_rule_dsl::facts::rows::InBlock> {
        if !self.has_parts() {
            return self.base.in_block();
        }
        if self.made.is_some() {
            return self.solved_in_block();
        }
        self.values.whole(&self.name).filter(|w| w.len() == self.len()).and_then(|w| w.in_block())
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_view(&self) -> Option<DeviceView> {
        if !self.has_parts() {
            return self.base.device_view();
        }
        if self.made.is_some() {
            return None;
        }
        self.view_over()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A source whose rows are made when counted, as `within`'s are, counting its counts.
    struct Counted(AtomicUsize, bool);
    impl Source for Counted {
        fn columns(&self) -> Vec<String> {
            vec!["n".into(), "a".into()]
        }
        fn len(&self) -> usize {
            self.0.fetch_add(1, Ordering::Relaxed);
            3
        }
        fn estimated_matches(&self, _: usize, _: bool) -> usize {
            1
        }
        fn indexed(&self, _: usize) -> bool {
            true
        }
        fn lookup(&self, _: usize, _: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
            Some(rerail_blocks::vector::Vector::new())
        }
        fn value(&self, row: usize, _: usize) -> i64 {
            row as i64
        }
        fn made_on_lookup(&self) -> bool {
            self.1
        }
    }

    /// A part is found by the host nodes its rows name, not by its host's document: a probe of another node of that
    /// document asks no part (t50: every `text` probe of a file with parsed strings asked all of them).
    #[test]
    fn a_hosts_node_finds_only_the_parts_that_name_it() {
        let values = Values::default();
        let (host, a, b) = (5i64 << 32, 10i64 << 32, 11i64 << 32);
        let rel = |rows: Vec<Vec<i64>>| Arc::new(super::super::Relation::from_rows(&["n", "s"], &mut rows.into_iter().map(|r| r.into_iter().collect()))) as Arc<dyn Source>;
        values.add(&[10], &[5], vec![("guest_of".to_owned(), rel(vec![vec![a | 1, host | 7], vec![a | 2, host | 7]]))]);
        values.add(&[11], &[5], vec![("guest_of".to_owned(), rel(vec![vec![b | 1, host | 9]]))]);
        let parts = values.parts.load().get("guest_of").cloned().expect("guest_of's parts");
        let asked = |v: i64| {
            let mut starts = Vec::new();
            parts.of(v, false, &mut |p: &Part| starts.push(p.start));
            starts
        };
        assert_eq!(asked(host | 7), vec![0], "the string its first part names");
        assert_eq!(asked(host | 9), vec![2], "the string its second part names");
        assert!(asked(host | 8).is_empty(), "a host node no part names");
        assert_eq!(asked(a | 2), vec![0], "a value's own node");
        assert_eq!(asked(b | 1), vec![2]);
        assert!(values.source_of("guest_of", host | 8).is_none(), "a host's node is the batch's, not a value's");
    }

    /// A layered source's read in place with parts is its lookup's rows with their values: the base's, then each part's
    /// that may hold the value, numbered past the base's.
    #[test]
    fn a_layered_read_in_place_is_its_lookups_rows() {
        let values = Arc::new(Values::default());
        let (host, a) = (5i64 << 32, 10i64 << 32);
        let rel = |rows: Vec<Vec<i64>>| Arc::new(super::super::Relation::from_rows(&["n", "s"], &mut rows.into_iter().map(|r| r.into_iter().collect()))) as Arc<dyn Source>;
        let base = rel(vec![vec![host | 1, 3], vec![host | 7, 4], vec![host | 7, 5]]);
        values.add(&[10], &[5], vec![("r".to_owned(), rel(vec![vec![a | 1, host | 7], vec![host | 7, 6]]))]);
        let layered = Layered::new("r", base, Arc::clone(&values));
        for (column, v) in [(0, host | 7), (0, a | 1), (1, host | 7), (0, host | 1), (0, host | 9), (1, 4)] {
            let mut read = Vec::new();
            assert!(layered.lookup_read(column, v, &[0, 1], &mut |r, row| read.push((r, row.to_vec()))), "read in place");
            let mut want: Vec<(usize, Vec<i64>)> = layered.lookup(column, &[v]).expect("a lookup").into_iter().map(|r| (r, vec![layered.value(r, 0), layered.value(r, 1)])).collect();
            read.sort();
            want.sort();
            assert_eq!(read, want, "column {column} = {v:#x}");
        }
    }

    /// An add makes the next parts from the last without copying them (R7's snapshots, 0494 step 5): only the last
    /// block of a relation's parts and the shard of the keys it adds are written again, the rest shared. Copied whole,
    /// each add copied every part and key before it: on corpus0 × B-now, 7.6 s of the run's 90 s.
    #[test]
    fn an_add_copies_only_its_last_block_and_its_keys_shard() {
        let values = Values::default();
        let part = || vec![("r".to_owned(), Arc::new(Counted(AtomicUsize::new(0), false)) as Arc<dyn Source>)];
        for k in 1..=200u32 {
            values.add(&[k], &[], part());
        }
        let before = values.parts.load().get("r").cloned().expect("r's parts");
        values.add(&[201], &[], part());
        let after = values.parts.load().get("r").cloned().expect("r's parts");
        let shared = |a: &[Arc<Vec<Part>>], b: &[Arc<Vec<Part>>]| a.iter().zip(b).filter(|(x, y)| Arc::ptr_eq(x, y)).count();
        assert_eq!(shared(&before.list.blocks, &after.list.blocks), before.list.blocks.len() - 1, "every block but the last shared");
        let copied = before.by_key.shards.iter().zip(&after.by_key.shards).filter(|(x, y)| !Arc::ptr_eq(x, y)).count();
        assert_eq!(copied, 1, "one key's shard written again");
        // Each value's rows are found by its key, and each row in its part.
        assert_eq!(after.list.len(), 201);
        for k in 1..=201u32 {
            assert!(values.source_of("r", i64::from(k) << 32).is_some(), "value {k}'s source");
        }
        let (p, row) = after.at(3 * 150 + 1).expect("row 451's part");
        assert_eq!((p.start, row), (450, 1));
        let (p, row) = after.at(3 * 200 + 2).expect("the last row's part");
        assert_eq!((p.start, row), (600, 2));
        assert!(after.at(3 * 201).is_none());
    }

    /// Rows `(n, d)`: a node id and a document key, looked up by filtering; read as found where `read`.
    struct Table {
        rows: Vec<[i64; 2]>,
        read: bool,
    }
    impl Source for Table {
        fn columns(&self) -> Vec<String> {
            vec!["n".into(), "d".into()]
        }
        fn len(&self) -> usize {
            self.rows.len()
        }
        fn estimated_matches(&self, _: usize, _: bool) -> usize {
            1
        }
        fn indexed(&self, _: usize) -> bool {
            true
        }
        fn lookup(&self, c: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
            Some((0..self.rows.len()).filter(|&r| values.contains(&self.rows[r][c])).collect())
        }
        fn value(&self, row: usize, c: usize) -> i64 {
            self.rows[row][c]
        }
        fn keys_documents(&self, c: usize) -> bool {
            c == 1
        }
        fn lookup_read(&self, c: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
            if !self.read {
                return false;
            }
            for r in self.lookup(c, &[value]).unwrap_or_default() {
                let row: Vec<i64> = columns.iter().map(|&k| self.rows[r][k]).collect();
                each(r, &row);
            }
            true
        }
    }

    /// A layered source's probe read as found (`lookup_read_at`) gives `lookup`'s rows, each with the values `value`
    /// reads, with values admitted: by a node id (its key's parts), by a document key (the parts of that document),
    /// and by a value no key names (every part), whether a part reads as found or not. Refused with values admitted,
    /// a chunk read every probe's rows a point read each.
    #[test]
    fn a_layered_probe_read_as_found_is_its_lookup_and_rows() {
        let id = |d: i64, l: i64| d << 32 | l;
        let base = Arc::new(Table { rows: vec![[id(1, 0), 1], [id(1, 1), 1], [id(2, 0), 2], [7, 2]], read: true }) as Arc<dyn Source>;
        let values = Arc::new(Values::default());
        for (k, read) in [(10u32, true), (11, false), (12, true)] {
            let k64 = i64::from(k);
            let part = Arc::new(Table { rows: vec![[id(k64, 0), k64], [id(k64, 1), k64], [7, k64]], read }) as Arc<dyn Source>;
            values.add(&[k], &[], vec![("r".to_owned(), part)]);
        }
        let layered = Layered::new("r", base, Arc::clone(&values));
        for (c, v) in [(0, id(1, 1)), (0, id(11, 1)), (0, id(12, 0)), (0, 7), (1, 2), (1, 11), (1, 12), (1, 99), (0, id(99, 0))] {
            let mut want: Vec<(usize, Vec<i64>)> = layered.lookup(c, &[v]).expect("a lookup").into_iter().map(|r| (r, vec![layered.value(r, 0), layered.value(r, 1)])).collect();
            let mut got = Vec::new();
            assert!(layered.lookup_read_at(c, v, &[0, 1], &mut super::super::super::ProbeCursor::default(), &mut |r, row| got.push((r, row.to_vec()))), "({c}, {v}) read");
            want.sort();
            got.sort();
            assert_eq!(got, want, "column {c}, value {v}");
            assert!(got.iter().all(|(_, row)| row[c] == v), "column {c}, value {v}: every row's");
        }
    }

    /// A scan of a source made on lookup (an axis) holds its values' rows past its own, and a lookup then reads the
    /// same rows (decision 0597): scanned, `has` over a Vue template's pieces held only the batch's rows, so a rule
    /// that planned the scan first found none of the pieces' nodes. A value admitted since adds its rows past them.
    #[test]
    fn a_scan_of_a_source_made_on_lookup_holds_its_values_rows() {
        let values = Arc::new(Values::default());
        let part = || vec![("r".to_owned(), Arc::new(Counted(AtomicUsize::new(0), false)) as Arc<dyn Source>)];
        let layered = Layered::new("r", Arc::new(Counted(AtomicUsize::new(0), true)) as Arc<dyn Source>, Arc::clone(&values));
        values.add(&[1], &[], part());
        assert_eq!(layered.len(), 6);
        let mut rows = Vec::new();
        layered.for_rows(&[0, 1], &mut |r, row| rows.push((r, row.to_vec())));
        let want: Vec<(usize, Vec<i64>)> = (0..6).map(|r| (r, vec![(r % 3) as i64; 2])).collect();
        assert_eq!(rows, want);
        assert_eq!((3..6).map(|r| layered.value(r, 1)).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(layered.lookup(0, &[1]), Some(rerail_blocks::vector![4]), "a lookup reads the scan's row");
        assert_eq!(layered.value(1 << 30, 0), (1 << 30) as i64, "a base's row past the scan's is the base's");
        values.add(&[2], &[], part());
        assert_eq!(layered.len(), 9);
        assert_eq!(layered.value(8, 0), 2);
        assert_eq!(layered.lookup(1, &[2]), Some(rerail_blocks::vector![5, 8]));
    }

    /// A value's long-text nodes, kept though it has none (a Vue piece's string reads its value in its own batch),
    /// answer for the value's own nodes alone, not its places' hosts' (no relation's part does, roll-up 95): a host
    /// file's node read went to a string annotation's batch and found nothing (corpus0: B009's `getattr(b, "x")` in a file with a quoted annotation).
    #[test]
    fn a_values_long_text_nodes_answer_for_its_own_nodes_alone() {
        let values = Values::default();
        let nodes = Arc::new(Counted(AtomicUsize::new(0), false)) as Arc<dyn Source>;
        // Another relation's row of the value's node and its host's node 3.
        let r = Arc::new(super::super::Relation::from_rows(&["n", "s"], &mut [[7i64 << 32 | 1, 1 << 32 | 3]].iter().map(|row| row.iter().copied().collect()))) as Arc<dyn Source>;
        values.add(&[7], &[1], vec![(super::super::LONG_TEXT_NODES.to_owned(), nodes), ("r".to_owned(), r)]);
        assert!(values.source_of(super::super::LONG_TEXT_NODES, 7 << 32).is_some(), "the value's own node");
        assert!(values.source_of(super::super::LONG_TEXT_NODES, 1 << 32 | 3).is_none(), "its host's node is the batch's");
        // Another relation's too: a host's node is read in the batch. Its rows naming the node are found by a lookup
        // at it (`Parts::of`, `hosted`).
        assert!(values.source_of("r", 1 << 32 | 3).is_none(), "its host's node is the batch's, for every relation");
        let parts = values.parts.load().get("r").cloned().expect("r's parts");
        let mut found = 0;
        parts.of(1 << 32 | 3, false, &mut |_: &Part| found += 1);
        assert_eq!(found, 1, "the rows naming its host's node, by a lookup there");
    }

    /// Every source a chunk binds is layered (decision 0431 §8), read or not: layering one counts none of its rows,
    /// and a source made on lookup is counted only by a scan's own `len` (PostHog: `within` made every row at each
    /// layering, 44 s of 246 s).
    #[test]
    fn layering_a_source_counts_none_of_its_rows() {
        for made_on_lookup in [false, true] {
            let base = Arc::new(Counted(AtomicUsize::new(0), made_on_lookup));
            let layered = Layered::new("r", Arc::clone(&base) as Arc<dyn Source>, Arc::new(Values::default()));
            assert_eq!(base.0.load(Ordering::Relaxed), 0, "made on lookup: {made_on_lookup}");
            assert_eq!(layered.value(1, 0), 1);
            let lookup = layered.lookup(0, &[1]);
            assert_eq!(lookup, Some(rerail_blocks::vector::Vector::new()));
            assert_eq!(base.0.load(Ordering::Relaxed), usize::from(!made_on_lookup), "made on lookup: {made_on_lookup}");
        }
    }
}
