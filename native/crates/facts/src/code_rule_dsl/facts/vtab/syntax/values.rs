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

/// One admitted value's rows of a relation: its source (a batch of the value alone), from row `start` of the parts.
#[derive(Clone)]
struct Part {
    start: usize,
    src: Arc<dyn Source>,
}

/// The first segment's length of an `AppendLog`; segment `s` holds `SEG0 << s`. Small: most relations' parts in a
/// chunk are a few.
const SEG0: usize = 8;
/// Segments of an `AppendLog`: room for `SEG0 * (2^32 - 1)` entries.
const SEGS: usize = 32;

/// Entries appended by one writer and read by any thread with no lock, each where it was written: segments that
/// double, made once, never moved. Shared by every snapshot (R7's, 0494 step 5), each reading below its own count,
/// none copied: a snapshot made from the last copied its last block of parts and its keys' shards (0.88 M on cold
/// PostHog).
/// Its segments' directory is made with its first entry: an empty log is a word.
struct AppendLog<T> {
    segs: std::sync::OnceLock<Box<[std::sync::OnceLock<Box<[T]>>; SEGS]>>,
}

impl<T: Default> Default for AppendLog<T> {
    fn default() -> Self {
        AppendLog { segs: std::sync::OnceLock::new() }
    }
}

impl<T: Default> AppendLog<T> {
    /// Entry `i`'s segment and place in it.
    fn place(i: usize) -> (usize, usize) {
        let s = (usize::BITS - 1 - (i / SEG0 + 1).leading_zeros()) as usize;
        (s, i - SEG0 * ((1 << s) - 1))
    }
    /// Entry `i`, its segment made if it is not (by the writer).
    fn slot(&self, i: usize) -> &T {
        let (s, at) = Self::place(i);
        let segs = self.segs.get_or_init(|| Box::new([const { std::sync::OnceLock::new() }; SEGS]));
        &segs[s].get_or_init(|| (0..SEG0 << s).map(|_| T::default()).collect())[at]
    }
    /// Entry `i`, where its segment is made (an entry a reader was given was written after it).
    fn get(&self, i: usize) -> Option<&T> {
        let (s, at) = Self::place(i);
        self.segs.get()?[s].get().map(|seg| &seg[at])
    }
}

/// The parts, in the order admitted: each set once, read by any snapshot below its own count (`len`).
#[derive(Clone, Default)]
struct Blocks {
    log: Arc<AppendLog<std::sync::OnceLock<Part>>>,
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
        (0..self.len).map(|i| &self[i])
    }
    /// Appends `part` (the one writer, `Values::add`, under its lock): a snapshot made before reads none of it.
    fn push(&mut self, part: Part) {
        assert!(self.log.slot(self.len).set(part).is_ok(), "a part set once: snapshots of one list are made one from the last");
        self.len += 1;
    }
    /// The first part for which `pred` is false, `pred` true of every part before it and false of every one after.
    fn partition_point(&self, pred: impl Fn(&Part) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if pred(&self[mid]) { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
}

impl<'a> IntoIterator for &'a Blocks {
    type Item = &'a Part;
    type IntoIter = Box<dyn Iterator<Item = &'a Part> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl std::ops::Index<usize> for Blocks {
    type Output = Part;
    fn index(&self, i: usize) -> &Part {
        assert!(i < self.len, "a part of the snapshot");
        self.log.get(i).and_then(std::sync::OnceLock::get).expect("a part below the snapshot's count is set")
    }
}

/// The parts that hold each key's rows: per key, the first link of its chain in an insert-only table, and each link
/// a part and the next link (0 at the last), in an append-only log. One writer (`Values::add`, under its lock)
/// appends; a reader walks a chain with no lock, the parts below its snapshot's count alone: a part admitted after
/// its snapshot is past its count, and a chain's parts rise. Shared by every snapshot, never copied.
/// Its log is made with its first key: a relation's parts found by no key of a kind (most have no `hosted`) hold none.
#[derive(Clone)]
struct Keys<K: ShardKey = u32> {
    log: Option<Arc<KeyLog>>,
    _k: std::marker::PhantomData<K>,
}

impl<K: ShardKey> Default for Keys<K> {
    fn default() -> Self {
        Keys { log: None, _k: std::marker::PhantomData }
    }
}

#[derive(Default)]
struct KeyLog {
    /// The keys' slots, replaced by a larger copy as it fills (an index's growth, as a vector's).
    table: arc_swap::ArcSwap<KeyTable>,
    /// Each link: its part in the high half, the next link in the low.
    links: AppendLog<AtomicU64>,
    /// The writer's counts: links and keys.
    counts: parking_lot::Mutex<(u32, usize)>,
}

/// A key's slot: its bits, its chain's first link (`EMPTY`: no key), and its last (the writer's).
#[derive(Default)]
struct Slot {
    key: AtomicU64,
    first: std::sync::atomic::AtomicU32,
    last: std::sync::atomic::AtomicU32,
}

/// No key in a slot.
const EMPTY: u32 = 0;

#[derive(Default)]
struct KeyTable {
    slots: Box<[Slot]>,
}

impl KeyTable {
    fn with_room(n: usize) -> KeyTable {
        let slots: Box<[Slot]> = (0..n).map(|_| Slot::default()).collect();
        KeyTable { slots }
    }
    fn hash(key: u64) -> usize {
        (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) as usize
    }
    /// Key `key`'s slot, or the empty slot it would take.
    fn find(&self, key: u64) -> Option<&Slot> {
        let mask = self.slots.len().checked_sub(1)?;
        let mut at = Self::hash(key) & mask;
        loop {
            let slot = &self.slots[at];
            // The first link set last, and read first: its key and link are the writer's before it.
            if slot.first.load(Ordering::Acquire) == EMPTY || slot.key.load(Ordering::Relaxed) == key {
                return Some(slot);
            }
            at = (at + 1) & mask;
        }
    }
}

/// A key's bits in a table.
trait ShardKey: Copy {
    fn bits(self) -> u64;
}
impl ShardKey for u32 {
    fn bits(self) -> u64 {
        u64::from(self)
    }
}
impl ShardKey for i64 {
    fn bits(self) -> u64 {
        self as u64
    }
}

impl<K: ShardKey> Keys<K> {
    /// The parts below `bound` that hold rows of key `k`, in the order admitted.
    fn parts(&self, k: K, bound: usize) -> impl Iterator<Item = usize> + '_ {
        // The table borrowed for the first link alone (`load`: no count of its holders written, as a probe of each
        // join would).
        let first = self.log.as_ref().and_then(|log| log.table.load().find(k.bits()).map(|s| s.first.load(Ordering::Acquire))).unwrap_or(EMPTY);
        // A link's number is one past its place (`EMPTY` is no link).
        let mut at = first;
        std::iter::from_fn(move || {
            let link = at.checked_sub(1)?;
            let word = self.log.as_ref()?.links.get(link as usize)?.load(Ordering::Acquire);
            let part = (word >> 32) as usize;
            if part >= bound {
                return None;
            }
            at = word as u32;
            Some(part)
        })
    }
    /// Part `i` holds rows of key `k` (the one writer).
    fn add(&mut self, k: K, i: usize) {
        let key = k.bits();
        let log = Arc::clone(self.log.get_or_insert_with(Arc::default));
        let mut counts = log.counts.lock();
        let table = log.table.load_full();
        if let Some(slot) = table.find(key)
            && slot.first.load(Ordering::Relaxed) != EMPTY
        {
            let last = slot.last.load(Ordering::Relaxed);
            let word = log.links.slot(last as usize - 1);
            let held = word.load(Ordering::Relaxed);
            if (held >> 32) as usize == i {
                return;
            }
            let link = Self::link(&log, &mut counts.0, i);
            word.store((held & !0xffff_ffff) | u64::from(link), Ordering::Release);
            slot.last.store(link, Ordering::Relaxed);
            return;
        }
        let link = Self::link(&log, &mut counts.0, i);
        // Room for the key at half full, else a table twice the size, its keys moved (they are the table's alone).
        let table = if (counts.1 + 1) * 2 > table.slots.len() {
            let grown = KeyTable::with_room((table.slots.len() * 2).max(8));
            for slot in table.slots.iter().filter(|s| s.first.load(Ordering::Relaxed) != EMPTY) {
                let to = grown.find(slot.key.load(Ordering::Relaxed)).expect("room");
                to.key.store(slot.key.load(Ordering::Relaxed), Ordering::Relaxed);
                to.last.store(slot.last.load(Ordering::Relaxed), Ordering::Relaxed);
                to.first.store(slot.first.load(Ordering::Relaxed), Ordering::Release);
            }
            let grown = Arc::new(grown);
            log.table.store(Arc::clone(&grown));
            grown
        } else {
            table
        };
        let slot = table.find(key).expect("room");
        slot.key.store(key, Ordering::Relaxed);
        slot.last.store(link, Ordering::Relaxed);
        slot.first.store(link, Ordering::Release);
        counts.1 += 1;
    }
    /// A new last link of part `i`, written before any chain names it: its number.
    fn link(log: &KeyLog, links: &mut u32, i: usize) -> u32 {
        let at = *links;
        // Its next is none (0: links are numbered from 1).
        log.links.slot(at as usize).store((i as u64) << 32, Ordering::Relaxed);
        *links += 1;
        at + 1
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
        for part in if docs { &self.by_doc } else { &self.by_key }.parts(key, self.list.len()) {
            each(&self.list[part]);
        }
        if !docs {
            for part in self.hosted.parts(v, self.list.len()) {
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
    parts: arc_swap::ArcSwap<FxHashMap<Arc<str>, Arc<Parts>>>,
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
        // Its parts' keys live with the chunk: made in the thread's long-lived room (decision 0342 amended 2026-11-24).
        // Made on whichever thread answered a request, on its stack they held it under them for the chunk.
        rerail_blocks::maps::long(|| self.add_within(own, hosts, sources))
    }

    fn add_within(&self, own: &[u32], hosts: &[u32], sources: Vec<(String, Arc<dyn Source>)>) {
        let _adding = self.adding.lock();
        // The next map from the last: its names shared, not made again (`Arc<str>`).
        let mut parts: FxHashMap<Arc<str>, Arc<Parts>> = (**self.parts.load()).clone();
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
            // value admitted together (`admit_values`, corpus.rs), the hosts apart (`hosts`): keyed by the first alone, a round's later
            // values' nodes were read in no batch (`Optional` of R10's second annotation).
            let (keyed, general) = if name == super::LONG_TEXT_NODES {
                (own, false)
            } else if src.rows_of_own_documents() && !own_docs.is_empty() {
                (&own_docs[..], false)
            } else {
                (own, true)
            };
            if !parts.contains_key(name.as_str()) {
                parts.insert(Arc::from(name.as_str()), Arc::default());
            }
            let p = Arc::make_mut(parts.get_mut(name.as_str()).expect("inserted"));
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
        let i = p.by_key.parts((n >> 32) as u32, p.list.len()).next()?;
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


/// Rows `from..` of a relation that holds a chunk's rows and then its values' (`Grower::admitted`): the rows of the
/// values admitted together, numbered from 0, read where they were written, looked up through the relation's index.
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
        true
    }
    /// The relation's rows of `values` (its index over every row, the chunk's and the values'), those of this slice:
    /// an answer's values are many, and read whole each probe read every one of them.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let range = self.from..self.from + self.len;
        let rows = self.whole.lookup(column, values)?;
        Some(rows.iter().copied().filter(|r| range.contains(r)).map(|r| r - self.from).collect())
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.whole.value(self.from + row, column)
    }
}

/// Where a layered source made on lookup numbers the values' rows a lookup reads (`Scan`): past every row containment
/// makes (`within`'s outer rows start at `1 << 31`, and a chunk makes far fewer than `1 << 30` of them), within the
/// `u32` the engine reads a row by. A scan of the source numbers them past the base's rows instead (`Made::at`).
const MADE_AT: usize = 0xF000_0000;
const MADE_ROOM: usize = (1 << 32) - MADE_AT;

/// A layered source made on lookup (containment, an axis) with values admitted: the values' rows, made whole once per
/// admission (`Layered::scanned`), each part's solve made once over its batch, and read without a lock. Made per
/// looked-up value under one lock, every probe of every thread took it (corpus0 × B-now: `within` 29 µs a lookup, and
/// a fifth of the run's CPU in the system's lock waits).
#[derive(Default)]
struct Made {
    scan: arc_swap::ArcSwapOption<Scan>,
    /// Held while the scan is made past the parts admitted since.
    extending: parking_lot::Mutex<()>,
    /// The base's count, where a scan numbers the values' rows past the base's: taken when a scan asks (a count of an
    /// axis makes every one of its rows, `has`'s; a lookup needs none).
    at: std::sync::OnceLock<usize>,
}

/// The values' rows of a layered source made on lookup (decision 0597: a rule over a Vue template's pieces,
/// `vcode(_, g, _), has(g, r, S)`, planned the axis's scan first, and the scan held none of the pieces' rows). Every
/// part's rows are made whole once; the parts admitted since add a segment past them, each written once where it is
/// made (decision 0353), a reader holding the segments it read. A lookup reads them by their index (`Seg::index`).
struct Scan {
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
    /// Row `i` of the scan's.
    fn row(&self, i: usize, width: usize) -> &[i64] {
        let k = self.segs.partition_point(|s| s.first <= i) - 1;
        let s = &self.segs[k];
        let at = (i - s.first) * width;
        &s.rows[at..at + width]
    }
    /// Every row, numbered from `at`.
    fn each(&self, at: usize, width: usize, f: &mut dyn FnMut(usize, &[i64]) -> bool) {
        for s in &self.segs {
            for (i, r) in s.rows.chunks(width).enumerate() {
                if !f(at + s.first + i, r) {
                    return;
                }
            }
        }
    }
    /// The rows whose `column` is `v`, numbered from `at`.
    fn holding(&self, at: usize, column: usize, v: i64) -> impl Iterator<Item = usize> + '_ {
        let key = [column as i64, v];
        self.segs.iter().flat_map(move |s| {
            let lo = s.index.partition_point(|e| [e[0], e[1]] < key);
            let hi = lo + s.index[lo..].partition_point(|e| [e[0], e[1]] == key);
            s.index[lo..hi].iter().map(move |e| at + s.first + e[2] as usize)
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
    /// For a base made on lookup, whose rows grow as it is read: the parts' rows, made whole once per admission.
    made: Option<Made>,
    width: usize,
    /// `has_parts`' last answer and the values admitted then (`admitted << 1 | has`): asked again only when more are.
    seen: AtomicU64,
}

impl Layered {
    pub fn new(name: &str, base: Arc<dyn Source>, values: Arc<Values>) -> Self {
        let made = base.made_on_lookup().then(Made::default);
        let width = base.columns().len();
        Layered { name: name.to_owned(), base, base_len: std::sync::OnceLock::new(), values, made, width, seen: AtomicU64::new(0) }
    }
    /// The batch's rows, where a value's rows are numbered past them (never for a base made on lookup, whose values'
    /// rows are at `MADE_AT`).
    fn base_len(&self) -> usize {
        *crate::code_rule_dsl::logic::profile::once(&self.base_len, "batch: a layered relation's rows", || self.base.len())
    }
    /// The batch's source.
    pub fn base(&self) -> &Arc<dyn Source> {
        &self.base
    }
    /// The source that holds node `n`'s document's rows: an admitted value's, else the batch's.
    pub fn source_of(&self, n: i64) -> Arc<dyn Source> {
        self.values.source_of(&self.name, n).unwrap_or_else(|| Arc::clone(&self.base))
    }
    /// SPIKE (`spike_strata`): each admitted value's batch, in the order admitted.
    pub fn value_batches(&self) -> Vec<Arc<super::Batch>> {
        self.values.batches.read().clone()
    }
    fn parts<T>(&self, f: impl FnOnce(&Parts) -> T) -> Option<T> {
        if self.values.admitted.load(Ordering::Acquire) == 0 {
            return None;
        }
        self.values.parts.load().get(self.name.as_str()).filter(|p| !p.list.is_empty()).map(|p| f(p))
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
    /// Made on lookup, with values admitted: the values' rows (`Scan`), made up to the parts admitted now; `f` reads
    /// them. Read without a lock once made: an admission's parts are made whole by the first reader after it.
    fn scanned<T>(&self, f: impl FnOnce(&Scan) -> T) -> Option<T> {
        let made = self.made.as_ref()?;
        if !self.has_parts() {
            return None;
        }
        let admitted = self.parts(|p| p.list.len()).unwrap_or(0);
        {
            // A guard, not a count of the scan's holders: every probe of every thread reads it.
            let scan = made.scan.load();
            if let Some(scan) = scan.as_ref().filter(|s| s.parts >= admitted) {
                return Some(f(scan));
            }
        }
        let width = self.width.max(1);
        let extending = made.extending.lock();
        let scan = made.scan.load_full().unwrap_or_else(|| Arc::new(Scan { parts: 0, segs: Vec::new() }));
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
        let scan = match more.flatten() {
            Some((parts, seg)) => {
                // The segments are shared, never their rows: the scan a reader holds keeps the segments it read.
                let mut next = Scan { parts, segs: scan.segs.clone() };
                next.segs.push(Arc::new(seg));
                assert!(next.len(width) <= MADE_ROOM, "{}: more rows made of the tape values than their room", self.name);
                let next = Arc::new(next);
                made.scan.store(Some(Arc::clone(&next)));
                next
            }
            None => scan,
        };
        // Read with the lock released: a reader may read this source again.
        drop(extending);
        Some(f(&scan))
    }
    /// Where a lookup numbers the values' rows: as a scan does once one has (a reader of both, a relation's version,
    /// takes a lookup's rows as the scan's), else at `MADE_AT`, no count of the base's made.
    fn lookup_at(&self) -> usize {
        self.made.as_ref().and_then(|m| m.at.get().copied()).unwrap_or(MADE_AT)
    }
    /// Where a scan of the source numbers the values' rows: past the base's, counted at the first scan.
    fn scan_at(&self) -> usize {
        self.made.as_ref().map_or(0, |m| *m.at.get_or_init(|| self.base.len()))
    }
    /// The values' row that row `row` of the source is, made on lookup: a lookup's (`MADE_AT`) or a scan's.
    fn made_row(&self, row: usize) -> Option<usize> {
        let made = self.made.as_ref()?;
        if row >= MADE_AT {
            return Some(row - MADE_AT);
        }
        let at = *made.at.get()?;
        let n = self.scanned(|s| s.len(self.width.max(1)))?;
        (at..at + n).contains(&row).then(|| row - at)
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
        // A placed base (`places::Places`: the view's rows, then its spans') is no view of one directory: its rows
        // and the values' are in no order one directory holds. Its `PlacedDir` is not the values' directory, which a
        // read of its kind would take for one.
        if v.kind & super::places::VIEW_PLACED != 0 {
            return None;
        }
        let dir = self.values.dir(&batch)?;
        let views = &dir.views;
        let loc = (super::device_chunk::VIEW_LOC_LINE..=super::device_chunk::VIEW_LOC_DOCOF).contains(&dv_ref.view);
        let (lines, locs) = views.holds();
        if (dv_ref.view == super::device_chunk::VIEW_LINE && !lines) || (loc && !locs) {
            return None;
        }
        // The base's kind, the directory of every document as the join programs here read it (the host's, where
        // they run on the host).
        *v = crate::code_rule_dsl::facts::vtab::ViewInPlace { kind: v.kind, ..views.read_of(dv_ref.view, Arc::clone(&dir) as Arc<dyn std::any::Any + Send + Sync>)? };
        dv_ref.rows = self.len();
        dv
    }

    /// An axis's rows on the device (a base made on lookup): the device's solve of the batch and of its values', off
    /// their tapes, in one relation, made again when values are admitted after it. Its rows are more than the source
    /// numbers (a value's are made on lookup, `MADE_AT`): a reader that binds it reads every pair.
    fn solved_in_block(&self) -> Option<crate::code_rule_dsl::facts::rows::InBlock> {
        let base = self.base.as_any()?.downcast_ref::<super::within::Within>()?;
        let parts = self.values.parts.load();
        let p = parts.get(self.name.as_str())?;
        let mut solved = self.values.solved.lock();
        if let Some((n, made)) = solved.get(self.name.as_str())
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
    /// chunk's or a tape value's. A value's parts are keyed by their hosts' keys too, so a host file's
    /// key names its guests' parts as well: the first of them that holds the node answers. The base is asked, then
    /// each part that holds `n`'s key, where they lie: no list of them is made.
    fn node_read(&self, name: &str, n: i64, more: &[i64]) -> Option<rusqlite::types::Value> {
        if let Some(v) = self.base.node_read(name, n, more) {
            return Some(v);
        }
        let parts = self.values.parts.load();
        let p = parts.get(self.name.as_str())?;
        p.by_key.parts((n >> 32) as u32, p.list.len()).find_map(|i| p.list[i].src.node_read(name, n, more))
    }
    /// `node_read`, the base's text lent; a part's is made (the parts are read under their list's guard).
    fn node_read_ref(&self, name: &str, n: i64, more: &[i64]) -> Option<super::super::NodeRead<'_>> {
        if let Some(v) = self.base.node_read_ref(name, n, more) {
            return Some(v);
        }
        let parts = self.values.parts.load();
        let p = parts.get(self.name.as_str())?;
        p.by_key.parts((n >> 32) as u32, p.list.len()).find_map(|i| p.list[i].src.node_read(name, n, more)).map(super::super::NodeRead::Value)
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
    /// Its base's, joined with all a tape value admitted later can hold in a column of its kind: any document's key
    /// (`1..2^31`: a value is a guest, its key past `GUEST_KEYS`, but a code guest's document and nodes are its
    /// file's, decision 0431 §4, a file this chunk need not hold). Not known where its base's column has no kind
    /// (`ColKind`): a value admitted later may hold anything there.
    fn col_bounds(&self, column: usize) -> Option<[i64; 2]> {
        use super::made::ColKind;
        let base = self.base.as_any();
        let kind = base
            .and_then(|b| b.downcast_ref::<super::Viewed>())
            .and_then(|v| v.col_kind(column))
            .or_else(|| base.and_then(|b| b.downcast_ref::<super::made::MadeRows>()).and_then(|m| m.col_kind(column)))
            .or_else(|| base.and_then(|b| b.downcast_ref::<super::within::Within>()).and_then(|w| w.col_kind(column)))?;
        let [lo, hi] = self.base.col_bounds(column)?;
        let keys = [1, (1i64 << 31) - 1];
        let [alo, ahi] = match kind {
            ColKind::Key => keys,
            ColKind::Node => [keys[0] << 32, (keys[1] << 32) | i64::from(u32::MAX)],
            ColKind::Atom | ColKind::U32 => [0, i64::from(u32::MAX)],
        };
        Some([lo.min(alo), hi.max(ahi)])
    }
    fn len(&self) -> usize {
        // Made on lookup, the base's rows grow as it is read; a scan's are every one of them, then the values' made
        // whole past them (`Scan`). A lookup's of a value not scanned are at `MADE_AT`.
        if self.made.is_some() {
            return self.scanned(|s| s.len(self.width.max(1))).map_or_else(|| self.base.len(), |n| self.scan_at() + n);
        }
        self.base_len() + self.values.total(&self.name)
    }
    fn estimated_len(&self) -> usize {
        self.base.estimated_len()
    }
    fn known_len(&self) -> Option<usize> {
        self.base.known_len()
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
    fn estimated_statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.base.estimated_statistics()
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
        let mut out = rerail_blocks::vector::Vector::new();
        self.lookup_each(column, values, &mut |r| out.push(r)).then_some(out)
    }
    /// The base's rows, then the parts' that may hold each value, numbered past the base's.
    fn lookup_each(&self, column: usize, values: &[i64], each: &mut dyn FnMut(usize)) -> bool {
        if !self.base.lookup_each(column, values, each) {
            return false;
        }
        if self.made.is_some() {
            let at = self.lookup_at();
            self.scanned(|s| values.iter().for_each(|&v| s.holding(at, column, v).for_each(&mut *each)));
            return true;
        }
        let docs = self.base.keys_documents(column);
        self.parts(|p| {
            for &v in values {
                p.of(v, docs, &mut |part| {
                    let at = self.base_len() + part.start;
                    if !part.src.lookup_each(column, &[v], &mut |r| each(at + r)) {
                        // A part that keeps no index of `column` is read whole: a value's rows are few.
                        (0..part.src.len()).filter(|&r| part.src.value(r, column) == v).for_each(|r| each(at + r));
                    }
                });
            }
        });
        true
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
        let Some(p) = all.get(self.name.as_str()).filter(|p| p.total > 0) else {
            return self.base.lookup_read_at(column, value, columns, cursor, each);
        };
        if columns.len() > 8 || !self.base.lookup_read_at(column, value, columns, cursor, each) {
            return false;
        }
        // Made on lookup: the values' rows of `value`, by the scan's index (`lookup`'s rows, as `value` reads them).
        if self.made.is_some() {
            let (width, at) = (self.width.max(1), self.lookup_at());
            self.scanned(|s| {
                let mut row = [0i64; 8];
                for i in s.holding(0, column, value) {
                    let r = s.row(i, width);
                    for (o, &c) in row.iter_mut().zip(columns) {
                        *o = r[c];
                    }
                    each(at + i, &row[..columns.len()]);
                }
            });
            return true;
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
    /// The base's test; where the relation has parts that may hold the value, `None` (the matches are read).
    fn test_at(&self, column: usize, value: i64, columns: &[usize], want: &[i64], cursor: &mut super::super::ProbeCursor) -> Option<bool> {
        if self.values.admitted.load(Ordering::Acquire) == 0 {
            return self.base.test_at(column, value, columns, want, cursor);
        }
        let all = self.values.parts.load();
        let Some(p) = all.get(self.name.as_str()).filter(|p| p.total > 0) else {
            return self.base.test_at(column, value, columns, want, cursor);
        };
        if self.made.is_some() {
            return None;
        }
        if self.base.test_at(column, value, columns, want, cursor)? {
            return Some(true);
        }
        // Each part that may hold the value, read as `lookup_read_at` reads it (a value's rows are few).
        let mut found = false;
        p.of(value, self.base.keys_documents(column), &mut |part| {
            if found {
                return;
            }
            let mut each = |_: usize, got: &[i64]| found |= got == want;
            if !part.src.lookup_read(column, value, columns, &mut each) {
                let rows = part.src.lookup(column, &[value]).unwrap_or_else(|| (0..part.src.len()).filter(|&r| part.src.value(r, column) == value).collect());
                found = rows.into_iter().any(|r| columns.iter().zip(want).all(|(&c, &w)| part.src.value(r, c) == w));
            }
        });
        Some(found)
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
            let at = self.scan_at();
            self.scanned(|s| {
                let mut row = vec![0i64; columns.len()];
                s.each(at, self.width.max(1), &mut |i, r| {
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
        if self.made.is_some() {
            return match self.made_row(row) {
                Some(i) => self.scanned(|s| s.row(i, self.width.max(1))[column]).unwrap_or(0),
                None => self.base.value(row, column),
            };
        }
        if row < self.base_len() {
            return self.base.value(row, column);
        }
        self.parts(|p| p.at(row - self.base_len()).map(|(part, r)| part.src.value(r, column))).flatten().unwrap_or(0)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        if self.made.is_some() {
            let Some(i) = self.made_row(row) else { return self.base.row_at(row, columns, out) };
            self.scanned(|s| {
                let r = s.row(i, self.width.max(1));
                for (o, &c) in out.iter_mut().zip(columns) {
                    *o = r[c];
                }
            });
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
            let at = self.scan_at();
            self.scanned(|s| {
                let mut row = vec![0i64; columns.len()];
                s.each(at, self.width.max(1), &mut |i, r| {
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
            if !self.has_parts() {
                return self.base.for_rows_in(range, columns, each);
            }
            let at = self.scan_at();
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
            let at = self.scan_at();
            self.scanned(|s| {
                s.each(at, self.width.max(1), &mut |i, r| {
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
        /// Its lookup's rows (none), read as found.
        fn lookup_read(&self, _: usize, _: i64, _: &[usize], _: &mut dyn FnMut(usize, &[i64])) -> bool {
            true
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

    /// An add makes the next parts from the last without copying them (R7's snapshots, 0494 step 5): the parts and
    /// the keys are logs every snapshot shares, each reading below its own count. Copied whole, each add copied every
    /// part and key before it: on corpus0 × B-now, 7.6 s of the run's 90 s; copied by block and shard, 0.88 M arrays
    /// on cold PostHog.
    #[test]
    fn an_add_copies_no_part_or_key_and_a_snapshot_reads_its_own() {
        let values = Values::default();
        let part = || vec![("r".to_owned(), Arc::new(Counted(AtomicUsize::new(0), false)) as Arc<dyn Source>)];
        for k in 1..=200u32 {
            values.add(&[k], &[], part());
        }
        let before = values.parts.load().get("r").cloned().expect("r's parts");
        values.add(&[201], &[], part());
        let after = values.parts.load().get("r").cloned().expect("r's parts");
        assert!(Arc::ptr_eq(&before.list.log, &after.list.log), "the parts shared");
        let same = |a: &Option<Arc<KeyLog>>, b: &Option<Arc<KeyLog>>| matches!((a, b), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
        assert!(same(&before.by_key.log, &after.by_key.log) && same(&before.by_doc.log, &after.by_doc.log), "the keys shared");
        assert!(before.hosted.log.is_none(), "no log of keys never added");
        // The snapshot before reads the parts it held, and none past them.
        assert_eq!(before.list.len(), 200);
        assert!(before.by_key.parts(201, before.list.len()).next().is_none(), "a part past its snapshot");
        assert_eq!(after.by_key.parts(201, after.list.len()).collect::<Vec<_>>(), [200]);
        assert_eq!(before.by_key.parts(7, before.list.len()).collect::<Vec<_>>(), [6]);
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

    /// A layered source made on lookup (an axis) makes its values' rows once an admission, whatever its lookups: each
    /// part read whole once (`Scan`), every lookup and read of a row then the scan's, without a lock. Made per
    /// looked-up value under one lock, every probe of every thread took it (corpus0 × B-now: `within` 29 µs a lookup).
    #[test]
    fn an_axis_s_values_rows_are_made_once_an_admission() {
        let values = Arc::new(Values::default());
        let layered = Layered::new("r", Arc::new(Counted(AtomicUsize::new(0), true)) as Arc<dyn Source>, Arc::clone(&values));
        let part = Arc::new(Counted(AtomicUsize::new(0), true));
        values.add(&[1], &[], vec![("r".to_owned(), Arc::clone(&part) as Arc<dyn Source>)]);
        let added = part.0.load(Ordering::Relaxed);
        for v in 0..100 {
            let rows = layered.lookup(0, &[v % 3]).expect("a lookup");
            assert_eq!(rows.len(), 1, "value {v}: the part's one row of it");
            assert_eq!(layered.value(rows[0], 0), v % 3);
            let mut read = Vec::new();
            assert!(layered.lookup_read_at(0, v % 3, &[0, 1], &mut super::super::super::ProbeCursor::default(), &mut |r, row| read.push((r, row.to_vec()))));
            assert_eq!(read, [(rows[0], vec![v % 3; 2])], "value {v}: read as found, as looked up");
        }
        assert_eq!(part.0.load(Ordering::Relaxed), added + 1, "the part read whole once");
        let other = Arc::new(Counted(AtomicUsize::new(0), true));
        values.add(&[2], &[], vec![("r".to_owned(), Arc::clone(&other) as Arc<dyn Source>)]);
        assert_eq!(layered.lookup(0, &[1]).expect("a lookup").len(), 2, "both parts' rows once the second is admitted");
        assert_eq!(part.0.load(Ordering::Relaxed), added + 1, "the first part not read again");
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

#[cfg(test)]
mod log_tests {
    use super::{AppendLog, Keys};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Entries land each in its segment, read back where written, past the first segments' boundaries.
    #[test]
    fn an_append_log_reads_each_entry_where_it_was_written() {
        let log = AppendLog::<AtomicU64>::default();
        for i in 0..10_000u64 {
            log.slot(i as usize).store(i * 3, Ordering::Relaxed);
        }
        assert!((0..10_000u64).all(|i| log.get(i as usize).is_some_and(|v| v.load(Ordering::Relaxed) == i * 3)));
        // Segments of 8, 16, 32, …: each entry in one, placed in order.
        assert_eq!(AppendLog::<AtomicU64>::place(7), (0, 7));
        assert_eq!(AppendLog::<AtomicU64>::place(8), (1, 0));
        assert_eq!(AppendLog::<AtomicU64>::place(23), (1, 15));
        assert_eq!(AppendLog::<AtomicU64>::place(24), (2, 0));
        assert!(AppendLog::<AtomicU64>::default().get(0).is_none(), "an empty log holds no segment");
    }

    /// A key's chain holds its parts in order, each once however often added, through the table's growth; a bound
    /// cuts it; a key never added has none. Readers on other threads walk it while it is written.
    #[test]
    fn keys_chain_their_parts_through_growth_and_concurrent_reads() {
        let mut keys = Keys::<i64>::default();
        for part in 0..300usize {
            for k in 0..(part as i64 % 40 + 1) {
                keys.add(k * 1_000_003, part);
                keys.add(k * 1_000_003, part);
            }
        }
        let want = |k: i64, bound: usize| (0..bound).filter(|p| (p % 40) as i64 >= k).collect::<Vec<_>>();
        for k in 0..40 {
            assert_eq!(keys.parts(k * 1_000_003, 300).collect::<Vec<_>>(), want(k, 300), "key {k}");
            assert_eq!(keys.parts(k * 1_000_003, 150).collect::<Vec<_>>(), want(k, 150), "key {k} below 150");
        }
        assert!(keys.parts(7, 300).next().is_none());
        // One writer, readers bounded by what it has written.
        // Made by its first key, as an add makes it, then shared by the readers.
        let mut first = Keys::<u32>::default();
        first.add(0, 0);
        let keys = Arc::new(first.clone());
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..3 {
                let (keys, done) = (Arc::clone(&keys), Arc::clone(&done));
                s.spawn(move || {
                    loop {
                        let bound = done.load(Ordering::Acquire);
                        for k in 0..64u32 {
                            let got: Vec<usize> = keys.parts(k, bound).collect();
                            let want: Vec<usize> = (0..bound).filter(|p| p % 64 >= k as usize).collect();
                            assert_eq!(got, want, "key {k} below {bound}");
                        }
                        if bound == 2_000 {
                            break;
                        }
                    }
                });
            }
            for part in 0..2_000usize {
                for k in 0..=(part % 64) as u32 {
                    first.add(k, part);
                }
                done.store(part + 1, Ordering::Release);
            }
        });
    }
}
