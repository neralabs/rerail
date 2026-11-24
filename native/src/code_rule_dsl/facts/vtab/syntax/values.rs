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

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use super::super::DeviceView;
use super::super::Source;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        if self.len % BLOCK == 0 {
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
struct Keys<K = u32> {
    shards: Vec<Arc<FxHashMap<K, Vec<usize>>>>,
}

impl<K> Default for Keys<K> {
    fn default() -> Self {
        Keys { shards: Vec::new() }
    }
}

/// A key's shard: its bits mixed, so that keys that differ in their low bits alone (a document's nodes) spread.
trait Shard: Copy + Eq + std::hash::Hash {
    fn shard(self) -> usize;
}
impl Shard for u32 {
    fn shard(self) -> usize {
        (self.wrapping_mul(0x9E37_79B9) >> 24) as usize % SHARDS
    }
}
impl Shard for i64 {
    fn shard(self) -> usize {
        ((self as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as usize % SHARDS
    }
}

impl<K: Shard> Keys<K> {
    fn shard(k: K) -> usize {
        k.shard()
    }
    fn get(&self, k: &K) -> Option<&Vec<usize>> {
        self.shards.get(Self::shard(*k))?.get(k)
    }
    /// Part `i` holds rows of key `k`.
    fn add(&mut self, k: K, i: usize) {
        if self.shards.is_empty() {
            self.shards = (0..SHARDS).map(|_| Arc::default()).collect();
        }
        if self.get(&k).is_some_and(|at| at.last() == Some(&i)) {
            return;
        }
        Arc::make_mut(&mut self.shards[Self::shard(k)]).entry(k).or_default().push(i);
    }
}

/// A relation's parts, in the order admitted, each found by the keys of the nodes it holds rows of. The index of the
/// parts' sources, not their rows: an add makes the next one from it (`Values::add`), sharing what it does not change.
#[derive(Clone, Default)]
struct Parts {
    list: Blocks,
    total: usize,
    by_key: Keys,
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
    /// The parts that may hold rows whose value is `v`: those whose own nodes are of `v`'s key, and those whose rows
    /// name node `v` of a document not theirs; every part for a value no key names (an atom, a number).
    fn of(&self, v: i64, each: &mut dyn FnMut(&Part)) {
        if !(1 << 32..).contains(&v) {
            self.list.iter().for_each(each);
            return;
        }
        for &i in self.by_key.get(&((v >> 32) as u32)).into_iter().flatten() {
            each(&self.list[i]);
        }
        for &i in self.hosted.get(&v).into_iter().flatten() {
            each(&self.list[i]);
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
    _batches: Vec<Arc<super::Batch>>,
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
        if let Some(b) = sources.iter().find_map(|(_, s)| s.python_batch()) {
            self.batches.write().push(b);
        }
        for (name, src) in sources {
            // A relation made on lookup (containment, an axis) has its rows only once a lookup asks.
            if src.len() == 0 && !src.made_on_lookup() {
                continue;
            }
            let p = Arc::make_mut(parts.entry(name).or_default());
            let i = p.list.len();
            // Its rows past the chunk's and the parts' before, in their block relation: the device binds that whole.
            let slice = src.as_any().and_then(|a| a.downcast_ref::<Slice>());
            p.sliced = (i == 0 || p.sliced) && slice.is_some_and(|sl| p.whole.as_ref().is_none_or(|w| w.len() == sl.from));
            p.whole = slice.filter(|_| p.sliced).map(|sl| Arc::clone(&sl.whole));
            p.list.push(Part { start: p.total, src: Arc::clone(&src) });
            p.total += src.len();
            for &k in own {
                p.by_key.add(k, i);
            }
            if src.made_on_lookup() {
                for &k in hosts {
                    p.by_key.add(k, i);
                }
            } else {
                let width = src.columns().len();
                let mut named: rustc_hash::FxHashSet<i64> = rustc_hash::FxHashSet::default();
                for r in 0..src.len() {
                    for c in 0..width {
                        let v = src.value(r, c);
                        if (1 << 32..).contains(&v) && !own.contains(&((v >> 32) as u32)) {
                            named.insert(v);
                        }
                    }
                }
                for v in named {
                    p.hosted.add(v, i);
                }
            }
        }
        self.parts.store(Arc::new(parts));
        // Released after the parts: a reader that sees the count sees them (`has_parts`).
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
        let i = *p.by_key.get(&((n >> 32) as u32))?.first()?;
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
        let made = super::device_chunk::views_over(batch, &batches).map(|views| Arc::new(Dir { views, _batches: batches.clone() }));
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
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
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
    rows: Vec<i64>,
    /// Each looked-up value's run of rows, and how many parts there were when it was made: a part admitted since (a
    /// value given a place more) makes it again.
    runs: FxHashMap<(usize, i64), ((usize, usize), usize)>,
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
}

impl Layered {
    pub fn new(name: &str, base: Arc<dyn Source>, values: Arc<Values>) -> Self {
        let made = base.made_on_lookup().then(|| parking_lot::Mutex::new(Made::default()));
        let width = base.columns().len();
        Layered { name: name.to_owned(), base, base_len: std::sync::OnceLock::new(), values, made, width }
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
    fn parts<T>(&self, f: impl FnOnce(&Parts) -> T) -> Option<T> {
        // No value admitted: no parts, and no load of them (`has_parts`).
        if self.values.admitted() == 0 {
            return None;
        }
        self.values.parts.load().get(&self.name).filter(|p| !p.list.is_empty()).map(|p| f(p))
    }
    /// With no value admitted, no relation has parts: a probe of a chunk with none (most of them) reads only the
    /// count, where the parts' load (`arc_swap`'s, a thread-local each) and the name's lookup took about 600 samples
    /// of a 15 s sample of `all`.
    fn has_parts(&self) -> bool {
        self.values.admitted() != 0 && self.values.total(&self.name) > 0
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Layered {
    /// A view's rows on the device: the chunk's batch's and its values', read in place through their directory. The
    /// base's view as the device reads it (its kind, its block), its directory that of every document.
    fn view_over(&self) -> Option<DeviceView<'_>> {
        let batch = self.base.python_batch()?;
        let mut dv = self.base.device_view();
        let dv_ref = dv.as_mut()?;
        // A view not read in place is made from the chunk's batch alone: it holds none of the values' rows.
        let v = dv_ref.in_place.as_mut()?;
        let dir = self.values.dir(&batch)?;
        let views = &dir.views;
        let loc = (super::device_chunk::VIEW_LOC_LINE..=super::device_chunk::VIEW_LOC_DOCOF).contains(&dv_ref.view);
        if (dv_ref.view == super::device_chunk::VIEW_LINE && !views.lines) || (loc && !views.locs) {
            return None;
        }
        v.dir = views.dir;
        v.buffers = views.buffers.clone();
        v.cache = views.cache.clone();
        v.keep = Arc::clone(&dir) as Arc<dyn std::any::Any + Send + Sync>;
        v.failed = v.cache_failed();
        dv_ref.rows = self.len();
        dv
    }

    /// An axis's rows on the device (a base made on lookup): the device's solve of the chunk's batch and of its
    /// values', in one relation, made again when values are admitted after it. Its rows are more than the source
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
        let made = more.and_then(|m| base.solved_with(&m));
        solved.insert(self.name.clone(), (p.list.len(), made.clone()));
        made.map(|w| w.in_block(w.len()))
    }
}

impl Source for Layered {
    fn python_batch(&self) -> Option<Arc<super::Batch>> {
        self.base.python_batch()
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
        // Made on lookup, the base's rows grow as it is read; a value's are past them (`MADE_AT`), never scanned.
        if self.made.is_some() {
            return self.base.len();
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
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.base.statistics()
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
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = self.base.lookup(column, values)?;
        if let Some(made) = &self.made {
            self.parts(|p| {
                let mut made = made.lock();
                for &v in values {
                    let run = match made.runs.get(&(column, v)) {
                        Some(&(run, parts)) if parts == p.list.len() => run,
                        _ => {
                            let at = made.rows.len() / self.width.max(1);
                            p.of(v, &mut |part| {
                                for r in part.src.lookup(column, &[v]).unwrap_or_default() {
                                    for c in 0..self.width {
                                        let x = part.src.value(r, c);
                                        made.rows.push(x);
                                    }
                                }
                            });
                            let run = (at, made.rows.len() / self.width.max(1) - at);
                            assert!(at + run.1 <= MADE_ROOM, "{}: more rows made of the tape values than its room", self.name);
                            made.runs.insert((column, v), (run, p.list.len()));
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
                p.of(v, &mut |part| {
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
        let mut more = Vec::new();
        self.parts(|p| {
            p.of(value, &mut |part| {
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
    /// The base's read, then each part that may hold `value` (`Parts::of`: its own nodes' key, or the host node its
    /// rows name), at its rows' place past the base's. Refused with parts made on lookup, and where the base has no
    /// such read (nothing given then). Refused whenever the chunk had parts, every probe of a chunk with a parsed string
    /// made a list of its rows (t50 step 6: `text` and the matchers' relations at about 0.6 µs a probe).
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if !self.has_parts() {
            return self.base.lookup_read(column, value, columns, each);
        }
        if self.made.is_some() || columns.len() > 8 || !self.base.lookup_read(column, value, columns, each) {
            return false;
        }
        self.parts(|p| {
            p.of(value, &mut |part| {
                let at = self.base_len() + part.start;
                if part.src.lookup_read(column, value, columns, &mut |r, row| each(at + r, row)) {
                    return;
                }
                let rows = part.src.lookup(column, &[value]).unwrap_or_else(|| (0..part.src.len()).filter(|&r| part.src.value(r, column) == value).collect());
                let mut row = [0i64; 8];
                for r in rows {
                    part.src.row_at(r, columns, &mut row[..columns.len()]);
                    each(at + r, &row[..columns.len()]);
                }
            })
        });
        true
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
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        if !self.has_parts() {
            return self.base.lookup_all(equal);
        }
        let &(c, v) = equal.first()?;
        let rows = self.lookup(c, &[v])?;
        Some(rows.into_iter().filter(|&r| equal.iter().all(|&(c, v)| self.value(r, c) == v)).collect())
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
        self.parts(|p| {
            p.of(v, &mut |part| {
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
        if !going || self.made.is_some() {
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
    fn lookup_eq(&self, column: usize, value: i64, eqs: &[(usize, i64)]) -> Option<Vec<usize>> {
        if self.has_parts() { None } else { self.base.lookup_eq(column, value, eqs) }
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        if self.has_parts() { None } else { self.base.lookup_in(column, values, doc) }
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.base.kept_to_doc(column)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(made) = &self.made {
            if row < MADE_AT {
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
            if row < MADE_AT {
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
            return self.base.for_rows_in(range, columns, each);
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
    fn device_view(&self) -> Option<DeviceView<'_>> {
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
        fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
            Some(Vec::new())
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
        let rel = |rows: Vec<Vec<i64>>| Arc::new(super::super::Relation::from_rows(&["n", "s"], &mut rows.into_iter())) as Arc<dyn Source>;
        values.add(&[10], &[5], vec![("guest_of".to_owned(), rel(vec![vec![a | 1, host | 7], vec![a | 2, host | 7]]))]);
        values.add(&[11], &[5], vec![("guest_of".to_owned(), rel(vec![vec![b | 1, host | 9]]))]);
        let parts = values.parts.load().get("guest_of").cloned().expect("guest_of's parts");
        let asked = |v: i64| {
            let mut starts = Vec::new();
            parts.of(v, &mut |p| starts.push(p.start));
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
        let rel = |rows: Vec<Vec<i64>>| Arc::new(super::super::Relation::from_rows(&["n", "s"], &mut rows.into_iter())) as Arc<dyn Source>;
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
            assert_eq!(lookup, Some(Vec::new()));
            assert_eq!(base.0.load(Ordering::Relaxed), usize::from(!made_on_lookup), "made on lookup: {made_on_lookup}");
        }
    }
}
