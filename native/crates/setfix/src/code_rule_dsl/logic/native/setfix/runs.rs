//! A relation's indexes (decision 0112): sorted orders of its row ids, only for the key sets its probes bind. An
//! order sorts the rows by its columns in turn (then by id), so every prefix of its columns is a range of it: one order
//! answers each key set that is a prefix of it, in any column order. The first order holds every column, so it also
//! answers "is this row already here?" (the dedup test): no hash set is kept.
//!
//! An order is a list of runs, one a wave: a wave's new rows are one run, its delta. A run holds 4 bytes an id and,
//! every 64 ids, the leading value there (its fences), so a probe searches the fences first, then at most two blocks
//! of ids. A probe that reads more than one run counts the runs past the first (the rent); at a wave's start the runs
//! merge into one when the rent since the last merge passes what the merge costs (the rent-or-buy rule). A frozen
//! relation's runs are merged two smallest at a time down to a few (`tier`): its probes cannot merge them later.
//!
//! ```text
//! // r(a, b, c), probed by (a) and by (b, c): two orders, each answering one key set and its prefixes
//! order [a, b, c]: run (wave 1) ids [7, 2, 9, …]  fences [a of id 7, a of the 64th id, …]   // (a), and dedup
//!                  run (wave 2) ids [12, 15]
//! order [b, c, a]: run (wave 1) …                                                          // (b), (b, c)
//! ```
//!
//! Runs are shared (`Arc`): a maintained relation started from a kept one takes its runs, not a copy.
//!
//! Rows a relation's writer knows to be in an order already (`Sorted`, decision 0180: a wave's batch is written
//! sorted by the first order) make runs in place: the rows `lo..hi` themselves, no ids listed and no sort.

use rerail_blocks::vector::Vector;
use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use super::rows::{Facts, Home, InRelations, Row};

/// A relation's rows, read by id: one flat vector (`[i64]`, `k` values a row), or the segments a maintained relation
/// shares with the kept one it started as (`super::Rows`, smoking gun 8). Every row lies whole in one segment.
/// Rows read by id: each read is a copy of the row's `k` values (`Row`, on the stack), never a reference into the
/// rows, so what holds them can be a block's columns (decision 0325).
pub(super) trait RowData {
    fn row(&self, id: u32, k: usize) -> Row;
    /// Row `id`'s value in column `c`: what a compare, a tag or a fence reads, not the row.
    fn value(&self, id: u32, k: usize, c: usize) -> i64;
    /// Where its indexes' arrays go (decision 0384 step 4): its relation's block; `None`, a run's open block.
    fn home(&self) -> Option<Home> {
        None
    }
    /// Whether a table lying in `facts` lies in its block (`home`'s): `false` once its rows moved to another (a
    /// tail past its block's room, `Rows::grow`), so the table moves after them. Read with no reference counted.
    fn lies_in(&self, _facts: &Arc<Facts>) -> bool {
        true
    }
}

impl RowData for [i64] {
    #[inline]
    fn row(&self, id: u32, k: usize) -> Row {
        Row::of(&self[id as usize * k..][..k])
    }
    #[inline]
    fn value(&self, id: u32, k: usize, c: usize) -> i64 {
        self[id as usize * k + c]
    }
}


pub(super) use rerail_blocks::order::{FENCE, gallop, print_span, print_vals, tag, tag_span};
use rerail_blocks::order::{print_of_key, split_at_pos};

/// A probe over a run costs about this many times the merge's cost of an id (O125: ~40 ns a run's probe, ~5 ns an
/// id merged).
const PROBE_PER_MERGED_ID: u64 = 8;

/// One run: row ids sorted by the order's columns (then id), and the leading value of every `FENCE`th id. A tagged
/// run (the order on every column, of two or more: the dedup test) sorts by 16 bits of the leading value's hash
/// (`tag`) first, then the columns in turn, and keeps each id's tag (`tags`) instead of fences: a key's ids are found
/// by its tag among the tags (placed by interpolation, as hash order spreads them evenly), and only the ids of that
/// tag are read. Equal leading values have equal tags, so every prefix of the columns is still a range.
#[derive(Clone)]
pub(super) struct Run {
    ids: Ids,
    fences: rerail_blocks::vector::Vector<i64>,
    tags: rerail_blocks::vector::Vector<u16>,
    /// In an order probed by whole keys (`Index::printed`): each id's key print (`print`), beside it. A whole key's
    /// ids are found among the prints of its window, a word of prints at a time, and only the first and last match
    /// read (`print_span`). Empty otherwise.
    prints: rerail_blocks::vector::Vector<u8>,
}

/// A run's ids: listed, or the rows `lo..hi` themselves, known to be in the order already (decision 0180), so no
/// list is kept (4 bytes a row less) and a probe searches the rows in place.
#[derive(Clone)]
enum Ids {
    List(rerail_blocks::vector::Vector<u32>),
    Rows(u32, u32),
}

impl Ids {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Ids::List(v) => v.len(),
            Ids::Rows(lo, hi) => (hi - lo) as usize,
        }
    }
    /// The id at position `i`.
    #[inline]
    fn get(&self, i: usize) -> u32 {
        match self {
            Ids::List(v) => v[i],
            Ids::Rows(lo, _) => lo + i as u32,
        }
    }
    /// Positions `s..e`.
    #[inline]
    fn part(&self, s: usize, e: usize) -> Part<'_> {
        match self {
            Ids::List(v) => Part::Ids(&v[s..e]),
            Ids::Rows(lo, _) => Part::Rows(lo + s as u32, lo + e as u32),
        }
    }
    fn iter(&self) -> PartIter<'_> {
        self.part(0, self.len()).iter()
    }
    #[cfg(test)]
    fn to_vec(&self) -> Vector<u32> {
        self.iter().collect()
    }
    fn bytes(&self) -> usize {
        match self {
            Ids::List(v) => v.capacity() * 4,
            Ids::Rows(..) => 0,
        }
    }
}

/// Some of a run's ids, in its order: listed, or the rows `lo..hi` (`Ids::Rows`).
#[derive(Clone, Copy)]
pub(super) enum Part<'a> {
    Ids(&'a [u32]),
    Rows(u32, u32),
}

impl Default for Part<'_> {
    fn default() -> Self {
        Part::Rows(0, 0)
    }
}

impl<'a> Part<'a> {
    #[inline]
    pub(super) fn len(self) -> usize {
        match self {
            Part::Ids(v) => v.len(),
            Part::Rows(lo, hi) => (hi - lo) as usize,
        }
    }
    #[inline]
    pub(super) fn is_empty(self) -> bool {
        self.len() == 0
    }
    /// Its first id (it is not empty).
    #[inline]
    pub(super) fn first(self) -> u32 {
        match self {
            Part::Ids(v) => v[0],
            Part::Rows(lo, _) => lo,
        }
    }
    #[inline]
    pub(super) fn iter(self) -> PartIter<'a> {
        match self {
            Part::Ids(v) => PartIter::Ids(v.iter()),
            Part::Rows(lo, hi) => PartIter::Rows(lo..hi),
        }
    }
}

/// One end of a range of values: the value, and whether it is in the range (`<=`, `>=`) or not (`<`, `>`).
pub(super) type End = Option<(i64, bool)>;

impl<'a> Part<'a> {
    /// Its ids whose column `c` lies between `lo` and `hi`, where they are sorted by it: a part a key found in an order
    /// whose next column is `c` (its rows equal on the key's columns are in `c`'s order). Two searches, no row walked.
    pub(super) fn within<D: RowData + ?Sized>(self, data: &D, k: usize, c: usize, lo: End, hi: End) -> Part<'a> {
        let id = |i: usize| match self {
            Part::Ids(v) => v[i],
            Part::Rows(l, _) => l + i as u32,
        };
        let v = |i: usize| data.value(id(i), k, c);
        let n = self.len();
        let s = lo.map_or(0, |(x, eq)| split_at_pos(0, n, |i| if eq { v(i) < x } else { v(i) <= x }));
        let e = hi.map_or(n, |(x, eq)| split_at_pos(s, n, |i| if eq { v(i) <= x } else { v(i) < x }));
        match self {
            Part::Ids(ids) => Part::Ids(&ids[s..e.max(s)]),
            Part::Rows(l, _) => Part::Rows(l + s as u32, l + e.max(s) as u32),
        }
    }
}

/// A part's ids in turn.
pub(super) enum PartIter<'a> {
    Ids(std::slice::Iter<'a, u32>),
    Rows(std::ops::Range<u32>),
}

impl Iterator for PartIter<'_> {
    type Item = u32;
    #[inline]
    fn next(&mut self) -> Option<u32> {
        match self {
            PartIter::Ids(it) => it.next().copied(),
            PartIter::Rows(r) => r.next(),
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            PartIter::Ids(it) => it.size_hint(),
            PartIter::Rows(r) => r.size_hint(),
        }
    }
}


/// A relation's row ranges known to be in an order already (decision 0180): each `lo..hi` sorted by its columns
/// (`cmp_vals`, tagged when they are every column). Its writer records them: a wave's new rows are written sorted by
/// the first order (`Fresh::settle`). A relation's rows are only appended (deleted ones stay in place, marked), so a
/// range stays true. Ranges in an order on columns `c` are in the order on any prefix of `c` too, untagged.
///
/// A frozen relation's ranges are in its block (`Relation::known`, decision 0478 step 5c): `base` reads them there,
/// holding its part. `own` are the ranges noted since (a relation's waves while it is derived), after `base`'s.
#[derive(Clone, Default)]
pub(super) struct Sorted {
    base: Option<crate::code_rule_dsl::facts::rows::Part>,
    own: Vec<(u32, u32, Arc<[usize]>)>,
}

/// Whether the sorts past a run's own use what is known sorted (decision 0187: a wave's batch, a scan for the next
/// step's probes, an order sorted again). `RERAIL_KNOWN_SORTED=runs` (a measurement aid) keeps it to the runs
/// (decision 0180); `off`, to nothing.
pub(super) fn known_beyond_runs() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_KNOWN_SORTED").map_or(true, |v| v != "runs" && v != "off"))
}

/// Whether rows in the order on `have` (relation of `k` columns) are in the order on `want` of the same relation.
fn implies(have: &[usize], want: &[usize], k: usize) -> bool {
    implies_key(have, k, tagged(want, k).then(|| want[0]), want)
}

/// Whether rows in the order on `have` (relation of `k` columns) are sorted by a key: the hash tag of column `tag`'s
/// value first (when given; it is `cols[0]`), then the values of `cols` in turn. A tagged `have` sorts by its leading
/// column's tag first, so only a tagged key on its prefix follows from it; an untagged one gives any untagged prefix.
fn implies_key(have: &[usize], k: usize, tag: Option<usize>, cols: &[usize]) -> bool {
    let prefix = cols.len() <= have.len() && have[..cols.len()] == *cols;
    match (tagged(have, k), tag) {
        _ if cols.is_empty() && tag.is_none() => true,
        (true, Some(t)) => prefix && have[0] == t,
        (false, None) => prefix,
        _ => false,
    }
}

impl Sorted {
    /// Rows `0..n` of `k` columns, all in the order on `cols` (a source whose rows lie in an order,
    /// `Source::order`), noted on its columns but the last where they are all `k`: an order on every column is the
    /// dedup order, which leads with a hash (`tagged`), and these lead with the values.
    pub(super) fn whole(n: u32, k: usize, cols: &[usize]) -> Sorted {
        let cols = if tagged(cols, k) { &cols[..k - 1] } else { cols };
        Sorted { base: None, own: if n > 0 && !cols.is_empty() { vec![(0, n, cols.into())] } else { Vec::new() } }
    }
    /// Rows `lo..hi` of `data` are in the order on `cols`: joined to the range before when it ends at `lo` in the
    /// same order and its last row is not past row `lo`.
    pub(super) fn note<D: RowData + ?Sized>(&mut self, data: &D, k: usize, cols: &[usize], lo: u32, hi: u32) {
        if hi <= lo {
            return;
        }
        debug_assert!((lo + 1..hi).all(|id| cmp_ids(data, k, cols, tagged(cols, k), id - 1, id).is_le()), "rows {lo}..{hi} said sorted on {cols:?} are not");
        if let Some((_, end, c)) = self.own.last_mut()
            && *end == lo
            && **c == *cols
            && cmp_ids(data, k, cols, tagged(cols, k), lo - 1, lo).is_le()
        {
            *end = hi;
            return;
        }
        self.own.push((lo, hi, cols.into()));
    }

    /// Every range, `base`'s then `own`'s, each given to `f` (lo, hi, its columns) until it says to stop.
    fn each(&self, f: &mut dyn FnMut(u32, u32, &[usize])) {
        if let Some(p) = &self.base {
            for kn in p.rel().known() {
                let mut c = [0usize; rerail_blocks::relation::MAX_KEY];
                let n = kn.ncols as usize;
                for (d, &s) in c.iter_mut().zip(&kn.cols[..n]) {
                    *d = s as usize;
                }
                f(kn.lo, kn.hi, &c[..n]);
            }
        }
        for (a, b, c) in &self.own {
            f(*a, *b, c);
        }
    }

    /// The ranges written in `part`'s relation's block (decision 0478 step 5c), and read there from now on: true when
    /// they are (they were already, or there are none), false when its block keeps others or has no room (they then
    /// stay as they were).
    pub(super) fn move_into_block(&mut self, part: &crate::code_rule_dsl::facts::rows::Part) -> bool {
        if self.own.is_empty() && self.base.as_ref().is_some_and(|b| std::ptr::eq(b.rel(), part.rel())) {
            return true;
        }
        if self.own.is_empty() && self.base.is_none() {
            return true;
        }
        let mut all: Vec<(u32, u32, Vector<u16>)> = Vec::new();
        let mut wide = false;
        self.each(&mut |a, b, c| {
            wide |= c.len() > rerail_blocks::relation::MAX_KEY;
            all.push((a, b, c.iter().map(|&x| x as u16).collect()));
        });
        if wide {
            return false;
        }
        let ranges: Vec<(u32, u32, &[u16])> = all.iter().map(|(a, b, c)| (*a, *b, &c[..])).collect();
        let written = {
            let _g = part.facts().ix.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: under its block's index lock, at the relation's freeze: nothing else writes its descriptor.
            unsafe { part.rel().try_set_known(&part.facts().block, &ranges) }
        };
        if written {
            self.base = Some(part.clone());
            self.own = Vec::new();
        }
        written
    }

    /// Whether every range is read in a block (none noted since).
    pub(super) fn in_block(&self) -> bool {
        self.own.is_empty()
    }
    /// The parts of rows `lo..hi` known to be in the order on `cols`, in row order.
    pub(super) fn within(&self, cols: &[usize], k: usize, lo: u32, hi: u32) -> Vec<(u32, u32)> {
        // `RERAIL_KNOWN_SORTED=off` (a measurement aid): nothing known, every run sorted.
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_KNOWN_SORTED").is_ok_and(|v| v == "off")) {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.each(&mut |a, b, c| if a < hi && b > lo && implies(c, cols, k) { out.push((a.max(lo), b.min(hi))) });
        out
    }
    /// The parts of rows `lo..hi` known to be sorted by a key (`implies_key`: column `tag`'s hash tag first, when
    /// given, then the values of `cols`), in row order.
    pub(super) fn within_key(&self, k: usize, tag: Option<usize>, cols: &[usize], lo: u32, hi: u32) -> Vec<(u32, u32)> {
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_KNOWN_SORTED").is_ok_and(|v| v == "off")) {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.each(&mut |a, b, c| if a < hi && b > lo && implies_key(c, k, tag, cols) { out.push((a.max(lo), b.min(hi))) });
        out
    }
    /// Rows known sorted, over every range.
    pub(super) fn rows(&self) -> u64 {
        let mut n = 0;
        self.each(&mut |a, b, _| n += u64::from(b - a));
        n
    }
}



/// A lab's counts of what probes read (`find_at`, every thread): [probes of a run, rows compared, tags or fences read].
#[cfg(test)]
pub(super) mod probe_reads {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
    /// Whether they count (a lab's counting rounds alone: counted, every compare writes one of three shared words).
    pub(in super::super) static ON: AtomicBool = AtomicBool::new(false);
    pub(in super::super) fn add(i: usize, n: u64) {
        if ON.load(Relaxed) {
            READS[i].fetch_add(n, Relaxed);
        }
    }
    /// The counts, zeroed.
    pub(in super::super) fn take() -> [u64; 3] {
        [0, 1, 2].map(|i| READS[i].swap(0, Relaxed))
    }
}

/// Whether an order on `cols` of a relation of `k` columns is tagged: the order on every column, of two or more.
#[inline]
pub(super) fn tagged(cols: &[usize], k: usize) -> bool {
    cols.len() >= 2 && cols.len() == k
}

/// Rows `ra` and `rb` (their values) in an order on `cols`: (tagged) the leading column's tag, then the columns in
/// turn.
#[inline]
pub(super) fn cmp_vals(cols: &[usize], tagged: bool, ra: &[i64], rb: &[i64]) -> Ordering {
    cmp_with(cols, tagged, |c| ra[c], |c| rb[c])
}

/// `cmp_vals` over two rows' values as read (`a(c)`, `b(c)`): only the columns compared are read.
#[inline]
pub(super) fn cmp_with(cols: &[usize], tagged: bool, a: impl Fn(usize) -> i64, b: impl Fn(usize) -> i64) -> Ordering {
    let Some(&lead) = cols.first() else { return Ordering::Equal };
    (if tagged { tag(a(lead)).cmp(&tag(b(lead))) } else { Ordering::Equal })
        .then_with(|| cols.iter().map(|&c| a(c).cmp(&b(c))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal))
}

/// Row `id` of `data` on `cols` against `key` (as long), each value read where it lies, until one differs.
#[inline]
fn cmp_key<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], key: &[i64], id: u32) -> Ordering {
    cols.iter().zip(key).map(|(&c, v)| data.value(id, k, c).cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// Rows `a` and `b` of `data` compared on `cols` (`cmp_vals`), each value read where it lies.
#[inline]
pub(super) fn cmp_ids<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], tagged: bool, a: u32, b: u32) -> Ordering {
    cmp_with(cols, tagged, |c| data.value(a, k, c), |c| data.value(b, k, c))
}


/// An order on the columns `cols` (every key set it answers is a prefix of them).
pub(super) struct Index {
    pub(super) cols: Vector<usize>,
    /// The runs, oldest first, each with the wave whose rows it holds (0: of no wave's delta).
    runs: Vec<(Arc<Run>, u32)>,
    /// Runs read past the first by probes since the last merge.
    rent: AtomicU64,
    /// The leading columns its runs keep their ids' prints of (`Run::prints`), 0 for none: a probe whose key holds them
    /// finds its rows by their prints (`rerail_blocks::order::within`).
    printed: usize,
}

impl Clone for Index {
    fn clone(&self) -> Index {
        Index { cols: self.cols.clone(), runs: self.runs.clone(), rent: AtomicU64::new(0), printed: self.printed }
    }
}

/// Rows `a` and `b` in the order on `cols` (`cmp_vals`), then by id (so a sort is the same whatever order it came in).
/// A merge gallops (`Run::galloped`) where one run has under 1/`GALLOP` of the other's ids.
const GALLOP: usize = 16;
/// `RERAIL_GALLOP=off`: every merge walks both runs, as before (to compare).
fn gallop_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| !matches!(std::env::var("RERAIL_GALLOP").as_deref(), Ok("off")))
}

#[inline]
fn cmp_rows<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], a: u32, b: u32) -> Ordering {
    cmp_ids(data, k, cols, tagged(cols, k), a, b).then_with(|| a.cmp(&b))
}

/// A run given to its relation's block (`Relation::try_add_order`): its rows listed or a range, its fences, tags and
/// prints, each a chunk of the block.
pub(super) struct GivenRun(pub(super) rerail_blocks::relation::GivenOrder);

impl GivenRun {
    /// The run again, its chunks its own (no directory took them).
    pub(super) fn into_run(self, facts: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>, life: crate::code_rule_dsl::facts::rows::Life) -> Run {
        use rerail_blocks::relation::{GivenOrder, OrderRows};
        let GivenOrder { rows, fences, tags, prints } = self.0;
        let ids = match rows {
            OrderRows::Range(lo, hi) => Ids::Rows(lo, hi),
            OrderRows::Listed(g) => Ids::List(crate::code_rule_dsl::facts::rows::from_chunk(facts, life, g)),
        };
        Run { ids, fences: crate::code_rule_dsl::facts::rows::from_chunk(facts, life, fences), tags: crate::code_rule_dsl::facts::rows::from_chunk(facts, life, tags), prints: crate::code_rule_dsl::facts::rows::from_chunk(facts, life, prints) }
    }
}

impl Run {
    /// Its arrays handed on to `facts`' block, where they lie (decision 0478 step 5b), or itself (boxed: it is the
    /// larger) when any lies elsewhere or is not its own.
    pub(super) fn give(self, facts: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>) -> Result<GivenRun, Box<Run>> {
        let ids_ok = match &self.ids {
            Ids::List(v) => v.can_give(facts),
            Ids::Rows(..) => true,
        };
        if !(ids_ok && self.fences.can_give(facts) && self.tags.can_give(facts) && self.prints.can_give(facts)) {
            return Err(Box::new(self));
        }
        fn take<T: rerail_blocks::Pod>(v: rerail_blocks::vector::Vector<T>, facts: &std::sync::Arc<crate::code_rule_dsl::facts::rows::Facts>) -> rerail_blocks::relation::Given {
            v.into_chunk(facts).unwrap_or_else(|_| unreachable!("checked"))
        }
        use rerail_blocks::relation::{GivenOrder, OrderRows};
        let Run { ids, fences, tags, prints } = self;
        let rows = match ids {
            Ids::List(v) => OrderRows::Listed(take(v, facts)),
            Ids::Rows(lo, hi) => OrderRows::Range(lo, hi),
        };
        Ok(GivenRun(GivenOrder { rows, fences: take(fences, facts), tags: take(tags, facts), prints: take(prints, facts) }))
    }

    /// The run order `i` of `owner`'s relation's directory is, read where it lies (borrowed: `rows::shared`).
    ///
    /// # Safety
    /// Order `i` is built in `owner`'s relation's directory, which `owner` keeps.
    pub(super) unsafe fn in_block(owner: &crate::code_rule_dsl::facts::rows::Part, i: usize) -> Run {
        use rerail_blocks::relation::ORDER_ROWS;
        let block = &owner.facts().block;
        let ix = owner.rel().index_at(i);
        let at = |p: *const u8| block.offset_of(p);
        // SAFETY: the caller's: each span is written in `owner`'s block, which its relation keeps.
        unsafe {
            let ids = if ix.ordered == ORDER_ROWS {
                Ids::Rows((ix.mask >> 32) as u32, (ix.mask & 0xffff_ffff) as u32)
            } else {
                Ids::List(crate::code_rule_dsl::facts::rows::shared(owner, at(ix.rows.as_ptr().cast()), ix.rows.len()))
            };
            let span = |len: usize, p: *const u8| if len == 0 { 0 } else { at(p) };
            Run {
                ids,
                fences: crate::code_rule_dsl::facts::rows::shared(owner, span(ix.fences.len(), ix.fences.as_ptr().cast()), ix.fences.len()),
                tags: crate::code_rule_dsl::facts::rows::shared(owner, span(ix.tags.len(), ix.tags.as_ptr().cast()), ix.tags.len()),
                prints: crate::code_rule_dsl::facts::rows::shared(owner, span(ix.prints.len(), ix.prints.as_ptr().cast()), ix.prints.len()),
            }
        }
    }

    /// Whether its ids are read where its relation's directory keeps them (`in_block`).
    fn in_place_of_block(&self) -> bool {
        match &self.ids {
            Ids::List(v) => v.is_shared(),
            Ids::Rows(..) => self.fences.is_shared() || self.tags.is_shared() || self.fences.is_empty(),
        }
    }

    fn fenced<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], ids: Ids) -> Run {
        let home = data.home();
        if tagged(cols, k) {
            let tags = crate::code_rule_dsl::facts::rows::collect_in(home.as_ref(), ids.iter().map(|id| tag(data.value(id, k, cols[0]))));
            return Run { ids, fences: crate::code_rule_dsl::facts::rows::vector_in(None, 0), tags, prints: crate::code_rule_dsl::facts::rows::vector_in(None, 0) };
        }
        let fences = match cols.first() {
            Some(&lead) => crate::code_rule_dsl::facts::rows::collect_in(home.as_ref(), ids.iter().step_by(FENCE).map(|id| data.value(id, k, lead))),
            None => crate::code_rule_dsl::facts::rows::vector_in(None, 0),
        };
        Run { ids, fences, tags: crate::code_rule_dsl::facts::rows::vector_in(None, 0), prints: crate::code_rule_dsl::facts::rows::vector_in(None, 0) }
    }

    /// The rows `lo..hi`, known to be in the order on `cols` (`Sorted`): no sort and no list of ids.
    fn in_place<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], lo: u32, hi: u32) -> Run {
        Run::fenced(data, k, cols, Ids::Rows(lo, hi))
    }

    /// This run with its ids' key prints (an order probed by whole keys): one row read an id.
    fn printed<D: RowData + ?Sized>(mut self, data: &D, k: usize, cols: &[usize]) -> Run {
        self.prints = crate::code_rule_dsl::facts::rows::collect_in(data.home().as_ref(), self.ids.iter().map(|id| print_vals(cols.iter().map(|&c| data.value(id, k, c)))));
        self
    }

    /// The ids of `ids` sorted by `cols` (then id), with their tag and first two values held beside them while
    /// they sort (`sort_keyed`).
    fn of<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], ids: impl IntoIterator<Item = u32>) -> Run {
        let home = data.home();
        let mut ids = crate::code_rule_dsl::facts::rows::collect_in(home.as_ref(), ids);
        let tagged = tagged(cols, k);
        let mut tags = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), if tagged { ids.len() } else { 0 });
        sort_keyed(&mut ids, tagged, cols.len(), |id, i| data.value(id, k, cols[i]), |a, b| cmp_rows(data, k, cols, a, b), &mut tags);
        if tagged {
            return Run { ids: Ids::List(ids), fences: crate::code_rule_dsl::facts::rows::vector_in(None, 0), tags, prints: crate::code_rule_dsl::facts::rows::vector_in(None, 0) };
        }
        Run::fenced(data, k, cols, Ids::List(ids))
    }

    /// The ids whose first `key.len()` columns of `cols` hold `key` (its prints, where it keeps them, of the first
    /// `printed`).
    #[inline]
    fn find<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], printed: usize, key: &[i64]) -> Part<'_> {
        // No columns: every row (a relation of no columns holds one row at most).
        if key.is_empty() {
            return self.ids.part(0, self.ids.len());
        }
        // The order's one search (`rerail_blocks::order::find`): the tags or fences, then the prints or a bisection.
        let (s, e) = rerail_blocks::order::find(self.ids.len(), &self.fences, &self.tags, &self.prints, printed, key, |i, m| cmp_key(data, k, &cols[..m], &key[..m], self.ids.get(i)));
        self.ids.part(s, e)
    }

    /// `find`, from where the last key of a sequence was found (`hint`, a position in this run, set past this key's
    /// ids): the tags or fences are searched galloping from there, either way, so a key near the last one (a sorted
    /// batch of keys, decision 0112) reads only what is near, and a far one costs about a search from scratch.
    #[inline]
    fn find_at<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], printed: usize, key: &[i64], hint: &mut u32) -> Part<'_> {
        let Some(&lead) = key.first() else { return self.ids.part(0, self.ids.len()) };
        let n = self.ids.len();
        let cmp = |i: usize, m: usize| -> Ordering {
            #[cfg(test)]
            probe_reads::add(1, 1);
            cmp_key(data, k, &cols[..m], &key[..m], self.ids.get(i))
        };
        #[cfg(test)]
        probe_reads::add(0, 1);
        let (lo, hi) = if !self.tags.is_empty() {
            let t = tag(lead);
            let s = gallop(&self.tags, (*hint as usize).min(n), |&x| { #[cfg(test)] probe_reads::add(2, 1); x < t });
            (s, s + gallop(&self.tags[s..], 0, |&x| { #[cfg(test)] probe_reads::add(2, 1); x == t }))
        } else {
            let a = gallop(&self.fences, (*hint as usize / FENCE).min(self.fences.len()), |&f| { #[cfg(test)] probe_reads::add(2, 1); f < lead });
            let b = a + gallop(&self.fences[a..], 0, |&f| { #[cfg(test)] probe_reads::add(2, 1); f == lead });
            (a.saturating_sub(1) * FENCE, (b * FENCE).min(n))
        };
        let (s, e) = rerail_blocks::order::within(lo, hi, &self.prints, printed, key, cmp);
        // Past the key's ids, or where it would be: the next key of a sorted batch is after it.
        *hint = if s < e { e } else { lo } as u32;
        self.ids.part(s, e)
    }

    /// Sets `held[i]` when a live id (`live`) holds row `order[i]` of `rows` (flat, `k` columns each): `order` lists
    /// distinct rows sorted as this run is (`cmp_vals`), so each row's search starts where the last one's ended. Its
    /// block is found by the fences (leading values, inline), or in a tagged run its tag's span by the tags; within it
    /// the search gallops, so rows near each other in the order read few rows of `data`.
    fn held_sorted<D: RowData + ?Sized, R: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], rows: &R, order: &[u32], live: &impl Fn(u32) -> bool, held: &mut [bool]) {
        let n = self.ids.len();
        let Some(&lead) = cols.first() else {
            // No columns: one row at most, held when a live id is.
            if self.ids.iter().any(live) {
                held.iter_mut().for_each(|h| *h = true);
            }
            return;
        };
        let tagged = !self.tags.is_empty();
        let mut at = 0;
        for (i, &q) in order.iter().enumerate() {
            if at >= n {
                break;
            }
            if held[i] {
                continue;
            }
            let row = rows.row(q, k);
            let cmp = |id: u32| -> Ordering { cmp_with(cols, tagged, |c| data.value(id, k, c), |c| row[c]) };
            if tagged {
                // The row's tag span, from where the last row's search ended; within it, the rows read in order.
                let (s, e) = tag_span(&self.tags, tag(row[lead]), at, false);
                let p = split_at_pos(s, e, |j| cmp(self.ids.get(j)).is_lt());
                at = p;
                held[i] = (p..e).map(|j| self.ids.get(j)).take_while(|&id| cmp(id).is_eq()).any(live);
                continue;
            }
            // The first id at or past the row is past the block before the first fence at or past its leading value.
            let v = row[lead];
            let from = at / FENCE;
            let f = from + self.fences[from..].partition_point(|&x| x < v);
            let start = at.max(f.saturating_sub(1) * FENCE);
            let mut bound = 1;
            while start + bound <= n && cmp(self.ids.get(start + bound - 1)).is_lt() {
                bound *= 2;
            }
            let lo = if bound > 1 { start + bound / 2 } else { start };
            let hi = (start + bound - 1).min(n);
            let p = split_at_pos(lo, hi, |j| cmp(self.ids.get(j)).is_lt());
            at = p;
            held[i] = (p..n).map(|j| self.ids.get(j)).take_while(|&id| cmp(id).is_eq()).any(live);
        }
    }

    /// Two runs as one. Two runs of rows in place that meet, the first's last row not past the second's first, are
    /// one run of rows in place (decision 0180): their tags and prints joined, no ids listed.
    fn merge<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], a: &Run, b: &Run) -> Run {
        if let (Ids::Rows(_, ahi), Ids::Rows(blo, _)) = (&a.ids, &b.ids) {
            let (first, second) = if ahi == blo { (a, b) } else { (b, a) };
            if let (Ids::Rows(lo, mid), Ids::Rows(mid2, hi)) = (&first.ids, &second.ids)
                && mid == mid2
                && (lo == mid || mid2 == hi || cmp_ids(data, k, cols, tagged(cols, k), mid - 1, *mid).is_le())
            {
                let home = data.home();
                let joined = |a: &[_], b: &[_]| {
                    let mut v = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), a.len() + b.len());
                    // copy: derived: two runs' prints joined into the merged run's (an index written where it is computed)
                    v.extend_from_slice(a);
                    // copy: derived: as the line above
                    v.extend_from_slice(b);
                    v
                };
                let prints = if !first.prints.is_empty() && !second.prints.is_empty() { joined(&first.prints, &second.prints) } else { crate::code_rule_dsl::facts::rows::vector_in(None, 0) };
                let ids = Ids::Rows(*lo, *hi);
                if tagged(cols, k) {
                    let tags = {
                        let mut v = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), first.tags.len() + second.tags.len());
                        // copy: derived: two runs' tags joined into the merged run's
                        v.extend_from_slice(&first.tags);
                        // copy: derived: as the line above
                        v.extend_from_slice(&second.tags);
                        v
                    };
                    return Run { ids, fences: crate::code_rule_dsl::facts::rows::vector_in(None, 0), tags, prints };
                }
                return Run { prints, ..Run::fenced(data, k, cols, ids) };
            }
        }
        let (an, bn) = (a.ids.len(), b.ids.len());
        // A small run into a large one (a maintained relation's few new rows into its kept ones): each small id is
        // placed by a binary search, and the large run's stretches between are copied whole. The same order as the
        // merge below (both are total, ties by id), at m log n row comparisons, not n + m.
        if gallop_on() && an.min(bn) * GALLOP < an.max(bn) {
            return if an >= bn { Run::galloped(data, k, cols, a, b) } else { Run::galloped(data, k, cols, b, a) };
        }
        Run::walked(data, k, cols, a, b)
    }

    /// `merge` by walking both runs, a row comparison per id.
    fn walked<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], a: &Run, b: &Run) -> Run {
        let (an, bn) = (a.ids.len(), b.ids.len());
        let home = data.home();
        let mut ids = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), an + bn);
        let tagged = !a.tags.is_empty() && !b.tags.is_empty();
        let mut tags = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), if tagged { an + bn } else { 0 });
        let printed = !a.prints.is_empty() && !b.prints.is_empty();
        let mut prints = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), if printed { an + bn } else { 0 });
        let (mut i, mut j) = (0, 0);
        while i < an && j < bn {
            let (x, y) = (a.ids.get(i), b.ids.get(j));
            // Tagged, the tags decide unless equal: rows are read only for ids of one tag.
            let first = if tagged && a.tags[i] != b.tags[j] { a.tags[i] < b.tags[j] } else { cmp_rows(data, k, cols, x, y).is_le() };
            if first {
                ids.push(x);
                if tagged { tags.push(a.tags[i]); }
                if printed { prints.push(a.prints[i]); }
                i += 1;
            } else {
                ids.push(y);
                if tagged { tags.push(b.tags[j]); }
                if printed { prints.push(b.prints[j]); }
                j += 1;
            }
        }
        ids.extend((i..an).map(|p| a.ids.get(p)));
        ids.extend((j..bn).map(|p| b.ids.get(p)));
        if printed {
            prints.extend_from_slice(&a.prints[i..]);
            prints.extend_from_slice(&b.prints[j..]);
        }
        if tagged {
            tags.extend_from_slice(&a.tags[i..]);
            tags.extend_from_slice(&b.tags[j..]);
            return Run { ids: Ids::List(ids), fences: crate::code_rule_dsl::facts::rows::vector_in(None, 0), tags, prints };
        }
        Run { prints, ..Run::fenced(data, k, cols, Ids::List(ids)) }
    }

    /// `merge` of a small run into a large one: each of `small`'s ids at the first place in `large` whose id
    /// orders after it, found by a binary search from the last place; `large`'s ids between are copied in stretches.
    fn galloped<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], large: &Run, small: &Run) -> Run {
        let (ln, sn) = (large.ids.len(), small.ids.len());
        let tagged = !large.tags.is_empty() && !small.tags.is_empty();
        let printed = !large.prints.is_empty() && !small.prints.is_empty();
        let home = data.home();
        let mut ids = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), ln + sn);
        let mut tags = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), if tagged { ln + sn } else { 0 });
        let mut prints = crate::code_rule_dsl::facts::rows::vector_in(home.as_ref(), if printed { ln + sn } else { 0 });
        // Whether `large`'s position `p` orders before `small`'s `q` (the merge's test).
        let before = |p: usize, q: usize| {
            if tagged && large.tags[p] != small.tags[q] { large.tags[p] < small.tags[q] } else { cmp_rows(data, k, cols, large.ids.get(p), small.ids.get(q)).is_le() }
        };
        let copy = |ids: &mut rerail_blocks::vector::Vector<u32>, tags: &mut rerail_blocks::vector::Vector<u16>, prints: &mut rerail_blocks::vector::Vector<u8>, s: usize, e: usize| {
            match &large.ids {
                Ids::List(v) => ids.extend_from_slice(&v[s..e]),
                Ids::Rows(lo, _) => ids.extend(lo + s as u32..lo + e as u32),
            }
            if tagged { tags.extend_from_slice(&large.tags[s..e]); }
            if printed { prints.extend_from_slice(&large.prints[s..e]); }
        };
        let mut at = 0;
        for q in 0..sn {
            let (mut lo, mut hi) = (at, ln);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if before(mid, q) { lo = mid + 1 } else { hi = mid }
            }
            copy(&mut ids, &mut tags, &mut prints, at, lo);
            ids.push(small.ids.get(q));
            if tagged { tags.push(small.tags[q]); }
            if printed { prints.push(small.prints[q]); }
            at = lo;
        }
        copy(&mut ids, &mut tags, &mut prints, at, ln);
        if tagged {
            return Run { ids: Ids::List(ids), fences: crate::code_rule_dsl::facts::rows::vector_in(None, 0), tags, prints };
        }
        Run { prints, ..Run::fenced(data, k, cols, Ids::List(ids)) }
    }

    /// Calls `f` on each group of ids equal on the first `m` columns of `cols`, in order.
    fn each_group<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], m: usize, mut f: impl FnMut(Part<'_>)) {
        let same = |a: u32, b: u32| cols[..m].iter().all(|&c| data.value(a, k, c) == data.value(b, k, c));
        let (n, mut at) = (self.ids.len(), 0);
        while at < n {
            let mut end = at + 1;
            while end < n && same(self.ids.get(at), self.ids.get(end)) {
                end += 1;
            }
            f(self.ids.part(at, end));
            at = end;
        }
    }
}

/// A key's candidate ids: its range of each run that holds it (the first few inline, so a probe allocates nothing).
#[derive(Clone, Default)]
pub(super) struct Found<'a> {
    first: [Part<'a>; 3],
    n: usize,
    rest: Vec<Part<'a>>,
    len: usize,
}

impl<'a> Found<'a> {
    fn add(&mut self, s: Part<'a>) {
        self.len += s.len();
        if self.n < self.first.len() {
            self.first[self.n] = s;
        } else {
            self.rest.push(s);
        }
        self.n += 1;
    }
    /// Its ids, over every run.
    pub(super) fn len(&self) -> usize {
        self.len
    }
    /// Its ranges, one a run.
    pub(super) fn parts(&self) -> impl Iterator<Item = Part<'a>> + '_ {
        self.first[..self.n.min(self.first.len())].iter().copied().chain(self.rest.iter().copied())
    }
    /// Its `i`th id, over every run in order (`i` under `len`): a walk samples candidates by position (decision 0236).
    pub(super) fn nth(&self, mut i: usize) -> u32 {
        for p in self.parts() {
            if i < p.len() {
                return match p { Part::Ids(v) => v[i], Part::Rows(lo, _) => lo + i as u32 };
            }
            i -= p.len();
        }
        panic!("a position past the found ids")
    }
    /// Its ids whose column `c` lies between `lo` and `hi`, each run's part narrowed (`Part::within`): found in an
    /// order whose column after the key's is `c`.
    pub(super) fn within<D: RowData + ?Sized>(&self, data: &D, k: usize, c: usize, lo: End, hi: End) -> Found<'a> {
        let mut out = Found::default();
        for p in self.parts() {
            let q = p.within(data, k, c, lo, hi);
            if !q.is_empty() {
                out.add(q);
            }
        }
        out
    }
    /// Its ids, over every run.
    pub(super) fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.parts().flat_map(Part::iter)
    }
}


thread_local! {
    /// How deep this thread is in builds other jobs of the pool may wait for (`inline`).
    static INLINE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Runs `f` with this thread's sorts and merges kept on it, off the pool: for a build other jobs may wait for (a
/// derived input's lazy order, `Csr::runs`, made inside its `OnceLock`). A wait for the pool's jobs runs the pool's
/// other jobs on the waiting thread (`threads::scope`); one that probes the same order then waits for the build on
/// the build's own stack, and never returns. hogql's cold runs hung so on a busy host: a join's morsel made an order
/// with a sort on the pool, and its wait ran another morsel of the join.
pub(super) fn inline<R>(f: impl FnOnce() -> R) -> R {
    struct Out;
    impl Drop for Out {
        fn drop(&mut self) {
            INLINE.with(|c| c.set(c.get() - 1));
        }
    }
    INLINE.with(|c| c.set(c.get() + 1));
    let _out = Out;
    f()
}

/// Whether a sort or merge here may go on the pool: more than one thread, and not inside an `inline` build.
fn on_pool() -> bool {
    crate::threads::max() > 1 && INLINE.with(|c| c.get() == 0)
}

/// What a sort or a merge works in: its keys and its merge's second buffer, in the run's blocks (`Vector`,
/// decision 0353: a join's scratch is the run's memory, not the heap's), or a caller's vector it sorts in place.
pub(super) trait Scratch<T: Copy>: std::ops::DerefMut<Target = [T]> + Sized {
    /// `n` items, each `v`.
    fn filled(n: usize, v: T) -> Self;
}

impl<T: Copy> Scratch<T> for Vec<T> {
    fn filled(n: usize, v: T) -> Self {
        vec![v; n]
    }
}

impl<T: rerail_blocks::Pod + Copy> Scratch<T> for Vector<T> {
    fn filled(n: usize, v: T) -> Self {
        let mut b = crate::code_rule_dsl::facts::rows::vector_in(None, n);
        b.resize(n, v);
        b
    }
}

/// `v`, sorted pieces one after another (`bounds`: each piece's start, then the end), merged into one sorted
/// sequence: adjacent pieces two at a time, a pass at a time, through one buffer of `v`'s length, `le(a, b)` saying
/// `a` may come before `b` (ties keep the left piece's first). Each item moves about log(pieces) times; one piece
/// moves nothing (decision 0187: rows known sorted are merged, not sorted again). From `PAR_ROWS` items, on the pool
/// (`merge_pieces_par`).
pub(super) fn merge_pieces<T: Copy + Send + Sync, V: Scratch<T>>(v: &mut V, mut bounds: Vector<usize>, le: impl Fn(T, T) -> bool + Sync) {
    if v.len() >= PAR_ROWS && on_pool() {
        return merge_pieces_par(v, bounds, le);
    }
    if bounds.len() <= 2 || v.is_empty() {
        return;
    }
    let mut buf = V::filled(v.len(), v[0]);
    while bounds.len() > 2 {
        let mut next = rerail_blocks::vector![0];
        let mut p = 0;
        let mut o = 0;
        while p + 1 < bounds.len() {
            let (a, b) = (bounds[p], bounds[p + 1]);
            let c = if p + 2 < bounds.len() { bounds[p + 2] } else { b };
            let (mut i, mut j) = (a, b);
            while i < b && j < c {
                if le(v[i], v[j]) { buf[o] = v[i]; i += 1 } else { buf[o] = v[j]; j += 1 }
                o += 1;
            }
            buf[o..o + b - i].copy_from_slice(&v[i..b]);
            o += b - i;
            buf[o..o + c - j].copy_from_slice(&v[j..c]);
            o += c - j;
            next.push(c);
            p += 2;
        }
        std::mem::swap(v, &mut buf);
        bounds = next;
    }
}

/// From this many items a sort or a merge of pieces runs on the pool (decision 0294); below it, a job costs more
/// than it saves.
pub(super) const PAR_ROWS: usize = 1 << 16;

/// The parts a level of a merge, or a sort, is cut into for each of the pool's threads.
const PARTS_PER_THREAD: usize = 4;

/// The fewest items a merge's part puts out: smaller pieces merge whole, a job each.
const PART_LEAST: usize = 1 << 13;

/// How many of `x`'s items are among the first `t` a merge of `x` and `y` puts out, where a tie takes `x`'s first
/// (merge path): the least `i` whose `x[i]` does not go before `y[t - i - 1]`.
fn co_rank<T: Copy>(x: &[T], y: &[T], t: usize, le: &impl Fn(T, T) -> bool) -> usize {
    let (mut lo, mut hi) = (t.saturating_sub(y.len()), t.min(x.len()));
    while lo < hi {
        let i = (lo + hi) / 2;
        let j = t - i;
        if j > 0 && le(x[i], y[j - 1]) { lo = i + 1 } else { hi = i }
    }
    lo
}

/// `merge_pieces` on the pool (decision 0294): each level's merges cut into parts by their outputs' places, each
/// part's inputs found by where its first and last outputs come from (`co_rank`), so a level spreads over the
/// threads whatever its pieces' sizes, its last one merge as well. The same order as one thread's.
fn merge_pieces_par<T: Copy + Send + Sync, V: Scratch<T>>(v: &mut V, mut bounds: Vector<usize>, le: impl Fn(T, T) -> bool + Sync) {
    let n = v.len();
    if bounds.len() <= 2 || n == 0 {
        return;
    }
    let all = crate::threads::max() * PARTS_PER_THREAD;
    let mut buf = V::filled(n, v[0]);
    while bounds.len() > 2 {
        // (a, b, c, t0, t1): pieces `a..b` and `b..c` merged, its outputs `t0..t1` (from `a`), in output order.
        let mut parts: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
        let mut next = rerail_blocks::vector![0];
        let mut p = 0;
        while p + 1 < bounds.len() {
            let (a, b) = (bounds[p], bounds[p + 1]);
            let c = if p + 2 < bounds.len() { bounds[p + 2] } else { b };
            let k = ((c - a) * all).div_ceil(n.max(1)).min((c - a) / PART_LEAST).max(1);
            parts.extend((0..k).map(|q| (a, b, c, (c - a) * q / k, (c - a) * (q + 1) / k)));
            next.push(c);
            p += 2;
        }
        let (src, le) = (&v[..], &le);
        let mut rest: &mut [T] = &mut buf[..];
        crate::threads::scope(|jobs| {
            for &(a, b, c, t0, t1) in &parts {
                let (out, tail) = std::mem::take(&mut rest).split_at_mut(t1 - t0);
                rest = tail;
                jobs.spawn("a merge's part", move |_| {
                    let (x, y) = (&src[a..b], &src[b..c]);
                    let (mut i, i1) = (co_rank(x, y, t0, le), co_rank(x, y, t1, le));
                    let (mut j, j1) = (t0 - i, t1 - i1);
                    for o in out.iter_mut() {
                        if i < i1 && (j >= j1 || le(x[i], y[j])) { *o = x[i]; i += 1 } else { *o = y[j]; j += 1 }
                    }
                });
            }
        });
        std::mem::swap(v, &mut buf);
        bounds = next;
    }
}

/// `v.sort_unstable()`, on the pool from `PAR_ROWS` items (decision 0294): parts sorted at once, then merged
/// (`merge_pieces`). Distinct keys (the packed sorts' hold their id) come out as one thread sorts them.
fn sort_unstable_par<T: Ord + Copy + Send + Sync, V: Scratch<T>>(v: &mut V) {
    let n = v.len();
    let threads = crate::threads::max();
    if n < PAR_ROWS || !on_pool() {
        v.sort_unstable();
        return;
    }
    let size = n.div_ceil(threads * PARTS_PER_THREAD).max(PAR_ROWS / PARTS_PER_THREAD);
    crate::threads::scope(|jobs| {
        for part in v.chunks_mut(size) {
            jobs.spawn("a sort's part", move |_| part.sort_unstable());
        }
    });
    let bounds: Vector<usize> = (0..n).step_by(size).chain(std::iter::once(n)).collect();
    merge_pieces(v, bounds, |a, b| a <= b);
}

/// `v` sorted, where the pieces `known` (position ranges, in order) are sorted already: the stretches between them
/// sorted by `sort` (each alone), then every piece merged (`merge_pieces`). The positions taken as known.
pub(super) fn sort_pieces<V: Scratch<u32>>(v: &mut V, known: &[(usize, usize)], mut sort: impl FnMut(&mut [u32]), le: impl Fn(u32, u32) -> bool + Sync) -> usize {
    let n = v.len();
    let mut bounds = rerail_blocks::vector![0];
    let (mut at, mut taken) = (0, 0);
    for &(a, b) in known {
        let (a, b) = (a.max(at), b.min(n));
        if a >= b {
            continue;
        }
        if at < a {
            sort(&mut v[at..a]);
            bounds.push(a);
        }
        bounds.push(b);
        taken += b - a;
        at = b;
    }
    if at < n {
        sort(&mut v[at..n]);
        bounds.push(n);
    }
    bounds.dedup();
    if bounds.last() != Some(&n) {
        bounds.push(n);
    }
    merge_pieces(v, bounds, le);
    taken
}

/// Runs an order's probes keep their place in (`Index::find_at`); past them, a run is searched from scratch.
pub(super) const HINTS: usize = 8;

/// `ids` sorted as `cmp` orders them, where `cmp` orders by the leading value's tag (`tagged`) and then the order's
/// columns in turn, `val(id, i)` being an id's value in the order's column `i` (of `ncols`): the ids are sorted with
/// their tag and first two values held beside them, read once a row, and rows are compared (`cmp`) only within a
/// stretch equal in all three. The sorted ids' tags are put in `tags` (none when untagged). Held beside the ids, the keys
/// are 24 bytes a row sorted, for the sort only; compared through the rows instead, each comparison read two
/// scattered rows (O125: 8 G instructions of the cold run's 346 G).
///
/// Packed (decision 0186): the tag, the first two values less their least, and the id, each in as few bits as its
/// range needs, side by side in one u64 (8 bytes a row) or u128 (16) when they fit; the 24-byte tuple otherwise.
/// Each field is ordered as its value, so the packed keys sort as the tuples do. `RERAIL_SORT_PACKED=off` keeps
/// the tuple (a measurement aid).
pub(super) fn sort_keyed(ids: &mut [u32], tagged: bool, ncols: usize, val: impl Fn(u32, usize) -> i64, cmp: impl Fn(u32, u32) -> Ordering, tags: &mut impl Extend<u16>) {
    if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SORT_PACKED").map_or(true, |v| v != "off"))
        && sort_packed(ids, tagged, ncols, &val, &cmp, tags)
    {
        return;
    }
    sort_tupled(ids, tagged, ncols, val, cmp, tags)
}

/// A sort's tags not kept (`sort_keyed` for an order of no run).
pub(super) struct NoTags;

impl Extend<u16> for NoTags {
    fn extend<I: IntoIterator<Item = u16>>(&mut self, _: I) {}
}

/// `sort_keyed` by packed keys, its tags put in `out`, or `false` (nothing done) when its fields need more than 128
/// bits.
fn sort_packed(ids: &mut [u32], tagged: bool, ncols: usize, val: &impl Fn(u32, usize) -> i64, cmp: &impl Fn(u32, u32) -> Ordering, out: &mut impl Extend<u16>) -> bool {
    let nf = usize::from(tagged) + ncols.min(2);
    if ids.len() < 2 || nf == 0 {
        return false;
    }
    let held = Held::of(ids, tagged, ncols.min(2), val);
    if nf == 1 {
        return sort_one_value(ids, held);
    }
    sort_fields(ids, held, tagged, ncols, cmp, out)
}

/// The bits a span of `span` needs.
fn bits(span: u64) -> u32 {
    64 - span.leading_zeros()
}

/// The held fields of each row, read once (`sort_packed`): its tag (when tagged), then its first two values, each
/// value's bits with the sign flipped, so the fields order as the signed values do; `nf` a row, beside ids of `wid`
/// bits.
struct Held {
    f: rerail_blocks::vector::Vector<u64>,
    nf: usize,
    wid: u32,
}

impl Held {
    fn of(ids: &[u32], tagged: bool, nv: usize, val: &impl Fn(u32, usize) -> i64) -> Held {
        let nf = usize::from(tagged) + nv;
        let flip = |v: i64| (v as u64) ^ (1 << 63);
        let mut f: rerail_blocks::vector::Vector<u64> = crate::code_rule_dsl::facts::rows::vector_in(None, ids.len() * nf);
        let mut top = 0u32;
        for &id in ids {
            top = top.max(id);
            let v0 = val(id, 0);
            if tagged {
                f.push(u64::from(tag(v0)));
            }
            f.push(flip(v0));
            if nv > 1 {
                f.push(flip(val(id, 1)));
            }
        }
        Held { f, nf, wid: bits(u64::from(top)) }
    }
}

/// One value (most orders): packed in place, `(v - least) << id bits | id`; `false` past 64 bits.
fn sort_one_value(ids: &mut [u32], held: Held) -> bool {
    let (mut f, wid) = (held.f, held.wid);
    let (lo, hi) = f.iter().fold((u64::MAX, 0), |(l, h), &v| (l.min(v), h.max(v)));
    if bits(hi - lo) + wid > 64 {
        return false;
    }
    for (k, &id) in f.iter_mut().zip(ids.iter()) {
        *k = (*k - lo) << wid | u64::from(id);
    }
    sort_unstable_par(&mut f);
    let mask = (1u64 << wid) - 1;
    for (slot, k) in ids.iter_mut().zip(&f) {
        *slot = (k & mask) as u32;
    }
    true
}

/// Two or three held fields, packed beside the id: `ids` sorted, rows equal in every held field ordered by `cmp` when
/// the order has more columns, and the tags (when tagged) put in `out`; `false` past 128 bits.
fn sort_fields(ids: &mut [u32], held: Held, tagged: bool, ncols: usize, cmp: &impl Fn(u32, u32) -> Ordering, out: &mut impl Extend<u16>) -> bool {
    let Some(layout) = Layout::of(&held) else { return false };
    match sort_by_keys(ids, held.f, &layout, ncols > 2 || tagged) {
        Keys::Narrow(keys) => tie_and_tag(ids, &keys, &layout, tagged, ncols, cmp, out),
        Keys::Wide(keys) => tie_and_tag(ids, &keys, &layout, tagged, ncols, cmp, out),
    }
    true
}

/// Sorted keys' ties ordered by `cmp` when the order has more columns than they hold, and their tags put in `out`.
fn tie_and_tag<K: Key>(ids: &mut [u32], keys: &[K], layout: &Layout, tagged: bool, ncols: usize, cmp: &impl Fn(u32, u32) -> Ordering, out: &mut impl Extend<u16>) {
    if ncols > 2 {
        order_ties(ids, keys, layout.wid, cmp);
    }
    if tagged {
        out.extend(keys.iter().map(|&k| layout.first(k.wide()) as u16));
    }
}

/// A packed key as a sort holds it: in a u64 when its fields fit one, else a u128's high and low words (which order
/// as the u128 does), each in the run's blocks.
trait Key: Copy + Ord + Send + Sync + rerail_blocks::Pod {
    fn wide(self) -> u128;
}

impl Key for u64 {
    fn wide(self) -> u128 {
        u128::from(self)
    }
}

impl Key for [u64; 2] {
    fn wide(self) -> u128 {
        u128::from(self[0]) << 64 | u128::from(self[1])
    }
}

/// A sort's packed keys, kept for its ties and tags (`sort_fields`).
enum Keys {
    Narrow(rerail_blocks::vector::Vector<u64>),
    Wide(rerail_blocks::vector::Vector<[u64; 2]>),
}

/// Each field's least and greatest value in `f`, rows of `nf`.
fn bounds(f: &[u64], nf: usize) -> ([u64; 3], [u64; 3]) {
    let (mut lo, mut hi) = ([u64::MAX; 3], [0u64; 3]);
    for row in f.chunks_exact(nf) {
        for (j, &v) in row.iter().enumerate() {
            lo[j] = lo[j].min(v);
            hi[j] = hi[j].max(v);
        }
    }
    (lo, hi)
}

/// Where each held field sits in a packed key: its width (`w`, the bits of its span) at `shift`, less its least
/// (`lo`); the first highest, the id lowest, in `wid` bits; `total` bits in all.
struct Layout {
    nf: usize,
    wid: u32,
    total: u32,
    lo: [u64; 3],
    w: [u32; 3],
    shift: [u32; 3],
}

impl Layout {
    /// The layout of `held`'s fields, or `None` when they need more than 128 bits.
    fn of(held: &Held) -> Option<Layout> {
        let (nf, wid) = (held.nf, held.wid);
        let (lo, hi) = bounds(&held.f, nf);
        let mut w = [0u32; 3];
        for j in 0..nf {
            w[j] = bits(hi[j] - lo[j]);
        }
        let total = w.iter().sum::<u32>() + wid;
        if total > 128 {
            return None;
        }
        let mut shift = [0u32; 3];
        let mut at = wid;
        for j in (0..nf).rev() {
            shift[j] = at;
            at += w[j];
        }
        Some(Layout { nf, wid, total, lo, w, shift })
    }

    /// The key of `row` (its held fields) and `id`. A zero-width field holds nothing: its shift may be the key's whole
    /// width.
    fn pack(&self, row: &[u64], id: u32) -> u128 {
        let mut k = u128::from(id);
        for (j, &v) in row.iter().enumerate() {
            if self.w[j] > 0 {
                k |= u128::from(v - self.lo[j]) << self.shift[j];
            }
        }
        k
    }

    /// The first field's value in key `k` (the tag, when tagged). Zero-width, it is every row's least, and its place,
    /// past the other fields and the id, is the key's whole width when they fill all 128 bits.
    fn first(&self, k: u128) -> u64 {
        if self.w[0] == 0 {
            return self.lo[0];
        }
        (k >> self.shift[0]) as u64 + self.lo[0]
    }

    /// The id's bits of a key.
    fn id_mask(&self) -> u128 {
        (1u128 << self.wid) - 1
    }
}

/// `ids` sorted by their keys (`layout`'s, of `f`'s rows in turn), each in a u64 when the fields fit one: the sorted
/// keys, kept when `keep`.
fn sort_by_keys(ids: &mut [u32], f: rerail_blocks::vector::Vector<u64>, layout: &Layout, keep: bool) -> Keys {
    if layout.total > 64 {
        return Keys::Wide(sort_wide(ids, f, layout));
    }
    let narrow = sort_narrow(ids, f, layout);
    Keys::Narrow(if keep { narrow } else { crate::code_rule_dsl::facts::rows::vector_in(None, 0) })
}

/// `sort_by_keys` with keys of at most 64 bits.
fn sort_narrow(ids: &mut [u32], f: rerail_blocks::vector::Vector<u64>, layout: &Layout) -> rerail_blocks::vector::Vector<u64> {
    let mut narrow: rerail_blocks::vector::Vector<u64> = crate::code_rule_dsl::facts::rows::collect_in(None, f.chunks_exact(layout.nf).zip(ids.iter()).map(|(row, &id)| layout.pack(row, id) as u64));
    drop(f);
    sort_unstable_par(&mut narrow);
    let mask = layout.id_mask();
    for (slot, &k) in ids.iter_mut().zip(&narrow) {
        *slot = (u128::from(k) & mask) as u32;
    }
    narrow
}

/// `sort_by_keys` with keys of more than 64 bits.
fn sort_wide(ids: &mut [u32], f: rerail_blocks::vector::Vector<u64>, layout: &Layout) -> rerail_blocks::vector::Vector<[u64; 2]> {
    let mut keys: rerail_blocks::vector::Vector<[u64; 2]> = crate::code_rule_dsl::facts::rows::collect_in(None, f.chunks_exact(layout.nf).zip(ids.iter()).map(|(row, &id)| {
        let k = layout.pack(row, id);
        [(k >> 64) as u64, k as u64]
    }));
    drop(f);
    sort_unstable_par(&mut keys);
    let mask = layout.id_mask();
    for (slot, k) in ids.iter_mut().zip(keys.iter()) {
        *slot = (k.wide() & mask) as u32;
    }
    keys
}

/// The stretches of sorted `keys` equal in every held field (all but the id's low `wid` bits): their `ids` ordered by
/// `cmp`.
fn order_ties<K: Key>(ids: &mut [u32], keys: &[K], wid: u32, cmp: &impl Fn(u32, u32) -> Ordering) {
    let mut at = 0;
    while at < keys.len() {
        let mut end = at + 1;
        while end < keys.len() && keys[end].wide() >> wid == keys[at].wide() >> wid {
            end += 1;
        }
        if end - at > 1 {
            ids[at..end].sort_unstable_by(|&a, &b| cmp(a, b));
        }
        at = end;
    }
}

/// `sort_keyed` by 24-byte tuples of the tag, the first two values and the id.
fn sort_tupled(ids: &mut [u32], tagged: bool, ncols: usize, val: impl Fn(u32, usize) -> i64, cmp: impl Fn(u32, u32) -> Ordering, out: &mut impl Extend<u16>) {
    // The tag, the first two values' bits with the sign flipped (so they order as the signed values do) and the id,
    // in the run's blocks.
    let flip = |v: i64| (v as u64) ^ (1 << 63);
    let mut keys: rerail_blocks::vector::Vector<[u64; 4]> = crate::code_rule_dsl::facts::rows::collect_in(None, ids.iter().map(|&id| {
        let v0 = if ncols > 0 { val(id, 0) } else { 0 };
        [u64::from(if tagged { tag(v0) } else { 0 }), flip(v0), flip(if ncols > 1 { val(id, 1) } else { 0 }), u64::from(id)]
    }));
    sort_unstable_par(&mut keys);
    for (slot, key) in ids.iter_mut().zip(keys.iter()) {
        *slot = key[3] as u32;
    }
    if ncols > 2 {
        let mut at = 0;
        while at < keys.len() {
            let mut end = at + 1;
            while end < keys.len() && keys[end][..3] == keys[at][..3] {
                end += 1;
            }
            if end - at > 1 {
                ids[at..end].sort_unstable_by(|&a, &b| cmp(a, b));
            }
            at = end;
        }
    }
    if tagged {
        out.extend(keys.iter().map(|k| k[0] as u16));
    }
}

impl Index {
    pub(super) fn new(cols: Vector<usize>) -> Index {
        Index { cols, runs: Vec::new(), rent: AtomicU64::new(0), printed: 0 }
    }

    /// Its runs as one run, merged where they are several (decision 0478 step 5b: a frozen relation's order is one
    /// sorted order in its block), with its columns and whether it is printed; `None` with no runs.
    pub(super) fn into_one<D: RowData + ?Sized>(mut self, data: &D, k: usize) -> Option<(Vector<usize>, usize, Run)> {
        let run = match self.runs.len() {
            0 => return None,
            1 => {
                let (r, _) = self.runs.pop().expect("a run");
                Arc::try_unwrap(r).unwrap_or_else(|r| (*r).clone())
            }
            n => self.merged_of(data, k, n),
        };
        Some((self.cols, self.printed, run))
    }

    /// An order of one run, `run` (one read in its relation's block: `Run::in_block`).
    pub(super) fn of_run(cols: Vector<usize>, printed: usize, run: Run) -> Index {
        Index { cols, runs: vec![(Arc::new(run), 0)], rent: AtomicU64::new(0), printed }
    }

    /// Whether every run is read where its relation's directory keeps it (`Run::in_block`).
    pub(super) fn borrowed(&self) -> bool {
        self.runs.iter().all(|(r, _)| r.in_place_of_block())
    }

    /// Its first run's first id's address (none: null), to tell where its ids lie.
    #[cfg(test)]
    pub(super) fn first_ids(&self) -> *const u32 {
        match self.runs.first().map(|(r, _)| &r.ids) {
            Some(Ids::List(v)) => v.as_ptr(),
            _ => std::ptr::null(),
        }
    }

    /// An order its probes give whole keys to (a derived input's own index, decision 0112): a byte a row more, each
    /// id's key print, so a probe reads the prints of its window, not its rows (`print_span`).
    pub(super) fn printed(cols: Vector<usize>) -> Index {
        let printed = cols.len();
        Index { printed, ..Index::new(cols) }
    }

    /// An order printed on its first `printed` columns (`printed`, a probe's whole key where a range column follows).
    pub(super) fn printed_on(cols: Vector<usize>, printed: usize) -> Index {
        Index { printed: printed.min(cols.len()), ..Index::new(cols) }
    }

    /// An order printed on its lead (a byte a row): every probe of it gives its lead, so within the lead's fence or
    /// tag span (in a tagged order, every key whose lead shares its tag: about 15 at a million keys) the lead's rows
    /// are found by their prints, and a longer key is bisected among them alone (t56 step 7 C). The relations' orders
    /// (`Rel::want`, the first order), which later strata probe as kept chains.
    pub(super) fn led(cols: Vector<usize>) -> Index {
        Index::printed_on(cols, 1)
    }

    /// A run of the rows `lo..hi` in this order, printed if the order is: the prints made in row order (the rows
    /// read in turn), then placed as the ids sort.
    fn run_of<D: RowData + ?Sized>(&self, data: &D, k: usize, lo: u32, hi: u32) -> Run {
        presorted::note(data, k, &self.cols, lo, hi, self.printed > 0);
        if self.printed == 0 {
            return Run::of(data, k, &self.cols, lo..hi);
        }
        let by_row: rerail_blocks::vector::Vector<u8> = crate::code_rule_dsl::facts::rows::collect_in(None, (lo..hi).map(|id| print_vals(self.cols[..self.printed].iter().map(|&c| data.value(id, k, c)))));
        let mut run = Run::of(data, k, &self.cols, lo..hi);
        run.prints = crate::code_rule_dsl::facts::rows::collect_in(data.home().as_ref(), run.ids.iter().map(|id| by_row[(id - lo) as usize]));
        run
    }

    /// A run of the rows `lo..hi`, known to be in this order (`Sorted`): no sort, no ids (printed if the order is).
    fn run_in_place<D: RowData + ?Sized>(&self, data: &D, k: usize, lo: u32, hi: u32) -> Run {
        presorted::known(u64::from(hi - lo), self.printed > 0);
        let run = Run::in_place(data, k, &self.cols, lo, hi);
        if self.printed > 0 { run.printed(data, k, &self.cols[..self.printed]) } else { run }
    }

    /// Whether the set `keys` is a prefix of this order's columns (in any order).
    pub(super) fn answers(&self, keys: &[usize]) -> bool {
        !keys.is_empty() && keys.len() <= self.cols.len() && keys.iter().all(|c| self.cols[..keys.len()].contains(c))
    }

    /// The rows `lo..hi` as a new run of wave `wave` (dead rows are indexed as any; readers skip them).
    pub(super) fn add_run<D: RowData + ?Sized>(&mut self, data: &D, k: usize, lo: usize, hi: usize, wave: u32) {
        self.add_run_in(data, k, lo, hi, wave, &[]);
    }

    /// `add_run`, where the rows of `known` (parts of `lo..hi`, in row order) are known to be in this order already
    /// (`Sorted::within`): those are runs of rows in place, the rest sorted, and the pieces merged into one run (a
    /// single piece in place stays one: no ids listed).
    pub(super) fn add_run_in<D: RowData + ?Sized>(&mut self, data: &D, k: usize, lo: usize, hi: usize, wave: u32, known: &[(u32, u32)]) {
        let (lo, hi) = (lo as u32, hi as u32);
        if lo >= hi {
            return;
        }
        let _t = presorted::Timed::start(self.printed > 0);
        // Every row in a known sorted range (a device fixpoint's rounds, each its own run): each range a run as it
        // lies, none merged here (a probe reads each; `settle` and `tier` merge them where probes pay for it).
        let mut ranges: Vec<(u32, u32)> = known.iter().map(|&(a, b)| (a.max(lo), b.min(hi))).filter(|(a, b)| a < b).collect();
        ranges.sort_unstable();
        let covered = ranges.first().is_some_and(|r| r.0 == lo) && ranges.last().is_some_and(|r| r.1 == hi) && ranges.windows(2).all(|w| w[0].1 == w[1].0);
        if covered && ranges.len() > 1 {
            for (a, b) in ranges {
                self.runs.push((Arc::new(self.run_in_place(data, k, a, b)), wave));
            }
            return;
        }
        let run = self.run_over(data, k, lo, hi, known);
        self.runs.push((Arc::new(run), wave));
    }

    /// One run of the rows `lo..hi`, those of `known` (in row order) taken in place, the rest sorted, the pieces merged.
    fn run_over<D: RowData + ?Sized>(&self, data: &D, k: usize, lo: u32, hi: u32, known: &[(u32, u32)]) -> Run {
        let mut pieces: Vec<Arc<Run>> = Vec::new();
        let mut at = lo;
        for &(a, b) in known {
            let (a, b) = (a.max(at), b.min(hi));
            if a >= b {
                continue;
            }
            if at < a {
                pieces.push(Arc::new(self.run_of(data, k, at, a)));
            }
            pieces.push(Arc::new(self.run_in_place(data, k, a, b)));
            at = b;
        }
        if at < hi {
            pieces.push(Arc::new(self.run_of(data, k, at, hi)));
        }
        Self::merged(data, k, &self.cols, pieces)
    }

    /// At a wave's start (before its run is added): merges the runs older than wave `wave` into one when the rent
    /// probes paid since the last merge (runs read past the first) passes what the merge costs.
    pub(super) fn settle<D: RowData + ?Sized>(&mut self, data: &D, k: usize, wave: u32) {
        let old = self.runs.iter().take_while(|(_, w)| *w < wave).count();
        if old < 2 {
            return;
        }
        let rent = *self.rent.get_mut();
        let ids: u64 = self.runs[..old].iter().map(|(r, _)| r.ids.len() as u64).sum();
        if rent * PROBE_PER_MERGED_ID <= ids {
            return;
        }
        *self.rent.get_mut() = 0;
        let merged = self.merged_of(data, k, old);
        let rest = self.runs.split_off(old);
        self.runs = vec![(Arc::new(merged), 0)];
        self.runs.extend(rest);
    }

    /// The first `n` runs merged into one.
    fn merged_of<D: RowData + ?Sized>(&self, data: &D, k: usize, n: usize) -> Run {
        Self::merged(data, k, &self.cols, self.runs[..n].iter().map(|(r, _)| Arc::clone(r)).collect())
    }

    /// `parts` merged into one: pairwise, smallest first (each id is copied about log(n) times); runs of rows in
    /// place that meet in order are joined first, as they merge without listing ids.
    fn merged<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], mut parts: Vec<Arc<Run>>) -> Run {
        let mut joined: Vec<Arc<Run>> = Vec::with_capacity(parts.len());
        for p in parts.drain(..) {
            if let Some(last) = joined.last()
                && matches!((&last.ids, &p.ids), (Ids::Rows(_, a), Ids::Rows(b, _)) if a == b)
            {
                let m = Run::merge(data, k, cols, last, &p);
                if matches!(m.ids, Ids::Rows(..)) {
                    *joined.last_mut().expect("a run") = Arc::new(m);
                    continue;
                }
            }
            joined.push(p);
        }
        parts = joined;
        while parts.len() > 1 {
            parts.sort_by_key(|r| std::cmp::Reverse(r.ids.len()));
            let a = parts.pop().expect("a run");
            let b = parts.pop().expect("a run");
            parts.push(Arc::new(Run::merge(data, k, cols, &a, &b)));
        }
        Arc::try_unwrap(parts.pop().expect("a run")).unwrap_or_else(|r| (*r).clone())
    }

    /// At most `max` runs, the two smallest merged at a time (a relation's frozen order: its probes cannot merge it
    /// later, and a large run is left alone while the small ones grow to its size).
    pub(super) fn tier<D: RowData + ?Sized>(&mut self, data: &D, k: usize, max: usize) {
        while self.runs.len() > max.max(1) {
            let mut by: Vector<usize> = (0..self.runs.len()).collect();
            by.sort_by_key(|&i| self.runs[i].0.ids.len());
            let (a, b) = (by[0].min(by[1]), by[0].max(by[1]));
            let merged = Run::merge(data, k, &self.cols, &self.runs[a].0, &self.runs[b].0);
            self.runs.remove(b);
            self.runs[a] = (Arc::new(merged), 0);
        }
    }

    /// The same rows ordered by `cols` instead, each run re-sorted (its wave kept).
    pub(super) fn reorder<D: RowData + ?Sized>(&mut self, data: &D, k: usize, cols: Vector<usize>, sorted: &Sorted) {
        if cols == self.cols {
            return;
        }
        self.cols = cols;
        let t = tagged(&self.cols, k);
        let mut runs = std::mem::take(&mut self.runs);
        for (r, _) in &mut runs {
            let lo = r.ids.iter().min().unwrap_or(0);
            // A run of rows `lo..hi` (a wave's, or runs of waves merged): the parts known sorted in the new order are
            // taken in place (decision 0187), the rest sorted.
            if r.ids.iter().max().is_some_and(|hi| (hi - lo) as usize + 1 == r.ids.len()) {
                let hi = lo + r.ids.len() as u32;
                presorted::site(presorted::REORDER, r.ids.len(), |i| cmp_ids(data, k, &self.cols, t, lo + i as u32 - 1, lo + i as u32).is_gt());
                let known = if known_beyond_runs() { sorted.within(&self.cols, k, lo, hi) } else { Vec::new() };
                presorted::site_known(presorted::REORDER, known.iter().map(|&(a, b)| u64::from(b - a)).sum());
                *r = Arc::new(self.run_over(data, k, lo, hi, &known));
                continue;
            }
            let run = Run::of(data, k, &self.cols, r.ids.iter());
            *r = Arc::new(if self.printed > 0 { run.printed(data, k, &self.cols[..self.printed]) } else { run });
        }
        self.runs = runs;
    }

    /// Every row is the delta of wave `wave` (a relation started from a kept one, all its rows new to the run).
    pub(super) fn restamp(&mut self, wave: u32) {
        for r in &mut self.runs {
            r.1 = wave;
        }
    }

    /// The ids whose first `key.len()` columns of this order hold `key` (in this order's column order): over every
    /// run, or only those of wave `wave` (the delta).
    #[inline]
    pub(super) fn find<'a, D: RowData + ?Sized>(&'a self, data: &D, k: usize, key: &[i64], delta: Option<u32>) -> Found<'a> {
        let mut found = Found::default();
        let mut read = 0u64;
        for (run, w) in &self.runs {
            if delta.is_some_and(|d| d != *w) {
                continue;
            }
            read += 1;
            let s = run.find(data, k, &self.cols, self.printed, key);
            if !s.is_empty() {
                found.add(s);
            }
        }
        if read > 1 {
            self.rent.fetch_add(read - 1, AtomicOrdering::Relaxed);
        }
        found
    }

    /// `find`, each run searched from where this sequence of probes last was in it (`hints`, one a run, set past this
    /// key's ids): a sorted batch of keys reads each run once, in order.
    #[inline]
    pub(super) fn find_at<'a, D: RowData + ?Sized>(&'a self, data: &D, k: usize, key: &[i64], delta: Option<u32>, hints: &mut [u32; HINTS]) -> Found<'a> {
        let mut found = Found::default();
        let mut read = 0u64;
        for (r, (run, w)) in self.runs.iter().enumerate() {
            if delta.is_some_and(|d| d != *w) {
                continue;
            }
            read += 1;
            let s = if r < HINTS { run.find_at(data, k, &self.cols, self.printed, key, &mut hints[r]) } else { run.find(data, k, &self.cols, self.printed, key) };
            if !s.is_empty() {
                found.add(s);
            }
        }
        if read > 1 {
            self.rent.fetch_add(read - 1, AtomicOrdering::Relaxed);
        }
        found
    }

    /// Whether this order is tagged (`tagged`) for a relation of `k` columns.
    pub(super) fn is_tagged(&self, k: usize) -> bool {
        tagged(&self.cols, k)
    }

    /// Sets `held[i]` when a live id (`live`) of this order holds row `order[i]` of `rows` (flat, `k` columns each):
    /// `order` lists distinct rows sorted by this order's columns, which are every column (the first order, the dedup
    /// test). Each run is searched once, in order (decision 0112: the dedup test a wave at a time).
    pub(super) fn held_sorted<D: RowData + ?Sized, R: RowData + ?Sized>(&self, data: &D, k: usize, rows: &R, order: &[u32], live: impl Fn(u32) -> bool, held: &mut [bool]) {
        debug_assert_eq!(self.cols.len(), k, "the dedup test is the order on every column");
        for (run, _) in &self.runs {
            run.held_sorted(data, k, &self.cols, rows, order, &live, held);
        }
        if self.runs.len() > 1 {
            self.rent.fetch_add((self.runs.len() as u64 - 1) * order.len() as u64, AtomicOrdering::Relaxed);
        }
    }

    /// Rows indexed.
    pub(super) fn rows(&self) -> usize {
        self.runs.iter().map(|(r, _)| r.ids.len()).sum()
    }

    /// Runs held.
    pub(super) fn runs(&self) -> usize {
        self.runs.len()
    }

    /// Calls `f` on each group of ids equal on the first `m` columns, over every run, in order (the runs merged for
    /// it when there are several; the index is not changed).
    pub(super) fn each_group<D: RowData + ?Sized>(&self, data: &D, k: usize, m: usize, f: impl FnMut(Part<'_>)) {
        match self.runs.as_slice() {
            [] => {}
            [(r, _)] => r.each_group(data, k, &self.cols, m, f),
            _ => self.merged_of(data, k, self.runs.len()).each_group(data, k, &self.cols, m, f),
        }
    }

    /// Whether every run of `self` is one of `other`'s, shared (`Arc`), not a copy.
    pub(super) fn shares(&self, other: &Index) -> bool {
        self.runs.iter().all(|(r, _)| other.runs.iter().any(|(o, _)| Arc::ptr_eq(r, o)))
    }

    /// Bytes held: an id a row, its tag (a tagged order), its print (a printed order), its fences, and each run's own allocation (shared runs
    /// counted by each holder).
    pub(super) fn bytes(&self) -> usize {
        self.runs.iter().map(|(r, _)| r.ids.bytes() + r.tags.capacity() * 2 + r.prints.capacity() + r.fences.capacity() * 8 + 64).sum::<usize>() + self.runs.capacity() * 16
    }
}

/// Measurement aid (`RERAIL_SPIKE_PRESORTED`): how much of each run's rows come already in the order's key order
/// (row id order), the case known-sorted ranges would skip the sort for.
pub mod presorted {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    /// [derived input orders, component orders] × [runs, rows, runs wholly in order, rows in stretches of 64 or more
    /// in order, stretches of 64 or more].
    /// And [other sorts' sites (`FRESH`, `SCAN`, `REORDER`)] × [sorts, rows, sorts wholly in order, rows in stretches
    /// of 64 or more in order, rows taken as known sorted]. The run's (decision 0442).
    #[derive(Default)]
    struct Counts {
        orders: [[AtomicU64; 7]; 2],
        sites: [[AtomicU64; 5]; 3],
    }
    fn counts() -> std::sync::Arc<Counts> {
        rerail_prof::code_rule_dsl::logic::profile::local::<Counts>()
    }
    /// Rows a run took in place, known sorted (decision 0180), into `orders[_][6]`, when measuring.
    pub(super) fn known(rows: u64, printed: bool) {
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_PRESORTED").is_some()) {
            counts().orders[usize::from(!printed)][6].fetch_add(rows, Relaxed);
        }
    }
    /// A run's build time (ns), into `orders[_][5]`, when measuring.
    pub(super) struct Timed(Option<(std::time::Instant, bool)>);
    impl Timed {
        pub(super) fn start(printed: bool) -> Timed {
            Timed(std::env::var_os("RERAIL_SPIKE_PRESORTED").is_some().then(|| (std::time::Instant::now(), printed)))
        }
    }
    impl Drop for Timed {
        fn drop(&mut self) {
            if let Some((t, printed)) = self.0 {
                counts().orders[usize::from(!printed)][5].fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
            }
        }
    }
    pub(super) fn note<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], lo: u32, hi: u32, printed: bool) {
        // `time`: the builds timed only, not counted (the count's pass over the rows would be timed too).
        if !rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SPIKE_PRESORTED").is_ok_and(|v| v != "time")) || hi <= lo {
            return;
        }
        let all = counts();
        let c = &all.orders[usize::from(!printed)];
        let (mut start, mut long, mut stretches, mut whole) = (lo, 0u64, 0u64, true);
        let mut close = |start: u32, end: u32| if end - start >= 64 { long += u64::from(end - start); stretches += 1; };
        for id in lo + 1..hi {
            if cmp_ids(data, k, cols, tagged(cols, k), id - 1, id) == Ordering::Greater {
                close(start, id);
                start = id;
                whole = false;
            }
        }
        close(start, hi);
        for (i, v) in [1, u64::from(hi - lo), u64::from(whole), long, stretches].into_iter().enumerate() {
            c[i].fetch_add(v, Relaxed);
        }
    }
    /// Other sorts: [a wave's batch (`Fresh::settle`), a scan for the next step's probes (`scan_order`), an order
    /// sorted again on other columns (`reorder`)] × [sorts, rows, sorts wholly in order, rows in stretches of 64 or
    /// more in order, rows taken as known sorted].
    pub const FRESH: usize = 0;
    pub const SCAN: usize = 1;
    pub const REORDER: usize = 2;
    fn counting() -> bool {
        rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SPIKE_PRESORTED").is_ok_and(|v| v != "time"))
    }
    /// A sort at `site` of `n` items, `descent(i)` telling item `i` is before item `i - 1` in the wanted order.
    pub fn site(site: usize, n: usize, descent: impl Fn(usize) -> bool) {
        if !counting() || n == 0 {
            return;
        }
        let (mut start, mut long, mut whole) = (0usize, 0u64, true);
        for i in 1..n {
            if descent(i) {
                if i - start >= 64 { long += (i - start) as u64; }
                start = i;
                whole = false;
            }
        }
        if n - start >= 64 { long += (n - start) as u64; }
        let all = counts();
        for (i, v) in [1, n as u64, u64::from(whole), long].into_iter().enumerate() {
            all.sites[site][i].fetch_add(v, Relaxed);
        }
    }
    /// Rows a sort at `site` took as known sorted, not sorted.
    pub fn site_known(site: usize, rows: u64) {
        if counting() {
            counts().sites[site][4].fetch_add(rows, Relaxed);
        }
    }
    pub fn report() -> String {
        let all = counts();
        let row = |i: usize| all.orders[i].iter().map(|a| a.load(Relaxed)).collect::<Vec<_>>();
        let (d, o) = (row(0), row(1));
        let sites = ["wave batches", "scans for the next probe", "orders sorted again"].iter().enumerate().map(|(i, name)| {
            let v: Vector<u64> = all.sites[i].iter().map(|a| a.load(Relaxed)).collect();
            format!("; {name}: {} sorts ({} wholly in order), {} rows, {} in order in stretches of 64+ ({:.0}%), {} known sorted", v[0], v[2], v[1], v[3], v[3] as f64 * 100.0 / v[1].max(1) as f64, v[4])
        }).collect::<String>();
        let f = |v: &[u64]| format!("{} runs ({} wholly in order), {} rows, {} in order in {} stretches of 64+ ({:.0}%); built in {:.1} ms; {} rows taken in place, known sorted", v[0], v[2], v[1], v[3], v[4], v[3] as f64 * 100.0 / v[1].max(1) as f64, v[5] as f64 / 1e6, v[6]);
        format!("derived-input orders: {}; component orders: {}{sites}", f(&d), f(&o))
    }
}

// The probe structures an index could be, over a run's captured relations (t50 step 5; measurement only).
#[cfg(test)]
mod index_spike;

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Inside a build other jobs may wait for (`inline`, a lazy order's), sorts and merges stay on the thread, nested
    /// builds too, and leave it as they found it, a panic included; outside, a large one goes on the pool. The sort's
    /// result is one thread's either way.
    #[test]
    fn a_build_others_wait_for_sorts_on_its_own_thread() {
        let pool = crate::threads::max() > 1;
        assert_eq!(on_pool(), pool);
        inline(|| {
            assert!(!on_pool());
            inline(|| assert!(!on_pool()));
            assert!(!on_pool());
            let mut v: Vector<u64> = (0..PAR_ROWS as u64 * 2).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20).collect();
            let mut want = v.clone();
            want.sort_unstable();
            sort_unstable_par(&mut v);
            assert_eq!(v, want);
        });
        assert_eq!(on_pool(), pool);
        assert!(std::panic::catch_unwind(|| inline(|| panic!("a build that fails"))).is_err());
        assert_eq!(on_pool(), pool);
    }

    /// Decision 0294: a merge of pieces on the pool puts out what one thread's does, ties included (the left piece's
    /// first: items equal by `le` and told apart by their ids), whatever the pieces' sizes; and a sort on the pool is
    /// one thread's.
    #[test]
    fn merges_and_sorts_on_the_pool_are_one_threads() {
        let mut x = 0x9E37_79B9u64;
        let mut next = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        // Ids whose key (`id / 3 % 1000`) ties often; pieces of very different sizes, each sorted by key.
        for sizes in [vec![200_000, 3, 70_000, 1], vec![1 << 17; 5], vec![90_000]] {
            let mut v: Vector<u32> = rerail_blocks::vector::Vector::new();
            let mut bounds = rerail_blocks::vector![0];
            for &n in &sizes {
                let mut piece: Vector<u32> = (0..n).map(|_| (next() % 4_000_000) as u32).collect();
                piece.sort_by_key(|&id| id / 3 % 1000);
                v.extend(piece);
                bounds.push(v.len());
            }
            let le = |a: u32, b: u32| a / 3 % 1000 <= b / 3 % 1000;
            let mut serial = v.clone();
            let mut b = bounds.clone();
            // One thread's merge: the loop `merge_pieces` runs below `PAR_ROWS`.
            while b.len() > 2 {
                let mut buf = rerail_blocks::vector::Vector::new();
                let mut nb = rerail_blocks::vector![0];
                for p in (0..b.len() - 1).step_by(2) {
                    let (lo, mid) = (b[p], b[p + 1]);
                    let hi = if p + 2 < b.len() { b[p + 2] } else { mid };
                    let (mut i, mut j) = (lo, mid);
                    while i < mid && j < hi { if le(serial[i], serial[j]) { buf.push(serial[i]); i += 1 } else { buf.push(serial[j]); j += 1 } }
                    buf.extend_from_slice(&serial[i..mid]);
                    buf.extend_from_slice(&serial[j..hi]);
                    nb.push(hi);
                }
                serial = buf;
                b = nb;
            }
            merge_pieces_par(&mut v, bounds, le);
            assert_eq!(v, serial, "pieces {sizes:?}");
        }
        let mut keys: Vector<u64> = (0..300_000).map(|_| next()).collect();
        let mut one = keys.clone();
        one.sort_unstable();
        sort_unstable_par(&mut keys);
        assert_eq!(keys, one);
    }

    /// Decision 0384 step 4: an order's runs over a relation's rows keep their arrays (ids, tags, fences, prints) in
    /// the relation's block, merged runs too, and answer as before.
    #[test]
    fn a_relations_runs_lie_in_its_block() {
        let k = 2;
        let mut rows = super::super::rows::Rows::new(k);
        let vals: Vector<i64> = (0..3000i64).flat_map(|i| [(i * 7919) % 101, i]).collect();
        // copy: not block data: a test's rows
        rows.extend_from_slice(&vals);
        let home = rows.home().expect("a home").facts().clone();
        let block = |v: Option<&Arc<super::super::rows::Facts>>| v.is_some_and(|f| Arc::ptr_eq(f, &home));
        for (ix, tagged) in [(Index::new(rerail_blocks::vector![0, 1]), true), (Index::printed(rerail_blocks::vector![0]), false)] {
            let mut ix = ix;
            ix.add_run(&rows, k, 0, 1000, 1);
            ix.add_run(&rows, k, 1000, 3000, 2);
            *ix.rent.get_mut() = u64::MAX / 16;
            ix.settle(&rows, k, 3);
            assert_eq!(ix.runs(), 1, "merged");
            let run = &ix.runs[0].0;
            let Ids::List(ids) = &run.ids else { panic!("ids listed") };
            assert!(block(ids.facts()), "ids in the relation's block");
            if tagged {
                assert!(block(run.tags.facts()), "tags in the relation's block");
            } else {
                assert!(block(run.fences.facts()) && block(run.prints.facts()), "fences and prints in the relation's block");
            }
            let want: Vector<u32> = (0..3000u32).filter(|&i| vals[i as usize * 2] == 5).collect();
            let mut got: Vector<u32> = ix.find(&rows, k, &[5], None).ids().collect();
            got.sort_unstable();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn orders_find_prefixes_merge_and_delta() {
        let k = 3;
        // Rows (i % 10, i % 7, i) for i in 0..2000: three waves of rows.
        let rows: Vector<i64> = (0..2000i64).flat_map(|i| [i % 10, i % 7, i]).collect();
        let mut by = Index::new(rerail_blocks::vector![0, 1, 2]);
        let mut by10 = Index::new(rerail_blocks::vector![1, 0, 2]);
        for (w, (lo, hi)) in [(0, 700), (700, 1500), (1500, 2000)].into_iter().enumerate() {
            by.add_run(&rows, k, lo, hi, w as u32 + 1);
            by10.add_run(&rows, k, lo, hi, w as u32 + 1);
        }
        assert!(by.answers(&[0]) && by.answers(&[1, 0]) && !by.answers(&[1]));
        assert_eq!(by.find(&rows, k, &[3], None).len(), 200);
        assert_eq!(by.find(&rows, k, &[3, 4], None).len(), 28);
        assert_eq!(by10.find(&rows, k, &[4, 3], None).len(), 28);
        assert_eq!(by.find(&rows, k, &[3, 4, 1243], None).len(), 1);
        // The delta: the last wave's rows of key 3.
        assert_eq!(by.find(&rows, k, &[3], Some(3)).len(), 50);
        assert_eq!(by.find(&rows, k, &[11], None).len(), 0);
        // Probed often enough, the runs older than wave 4 merge; a key's rows stay the same.
        for _ in 0..1000 {
            by.find(&rows, k, &[5, 5], None);
        }
        by.settle(&rows, k, 4);
        assert_eq!(by.runs(), 1);
        assert_eq!(by.find(&rows, k, &[5, 5], None).len(), 29);
        let (mut groups, mut seen) = (0, 0);
        by.each_group(&rows, k, 1, |ids| {
            groups += 1;
            seen += ids.len();
        });
        assert_eq!((groups, seen), (10, 2000));
        // Rarely probed, they stay apart; a delta probe reads one run and pays no rent.
        for _ in 0..1000 {
            by10.find(&rows, k, &[5], Some(3));
        }
        by10.settle(&rows, k, 4);
        assert_eq!(by10.runs(), 3);
        by10.tier(&rows, k, 2);
        assert_eq!(by10.runs(), 2);
        assert_eq!(by10.find(&rows, k, &[3], None).len(), (0..2000).filter(|i| i % 7 == 3).count());
    }

    /// A key's rows in an order whose next column is `c` narrowed to a range of `c` (`Found::within`) are the rows a
    /// filter of the key's rows keeps, over several runs, each end in the range or not, one end or both.
    #[test]
    fn a_keys_rows_in_a_range_of_the_next_column_are_found_by_searching() {
        let k = 3;
        let rows: Vector<i64> = (0..2000u32).flat_map(|i| [i64::from(i % 10), i64::from((i * 7) % 300), i64::from(i)]).collect();
        let mut by = Index::new(rerail_blocks::vector![0, 1, 2]);
        for (w, (lo, hi)) in [(0, 700), (700, 1500), (1500, 2000)].into_iter().enumerate() {
            by.add_run(&rows, k, lo, hi, w as u32 + 1);
        }
        let value = |id: u32, c: usize| rows[id as usize * k + c];
        for (lo, hi) in [(Some((40, false)), Some((90, false))), (Some((40, true)), Some((90, true))), (None, Some((5, false))), (Some((295, false)), None), (Some((90, false)), Some((40, false)))] {
            let found = by.find_at(&rows, k, &[3], None, &mut [0; HINTS]);
            let mut got: Vector<u32> = found.within(&rows, k, 1, lo, hi).ids().collect();
            got.sort_unstable();
            let inside = |v: i64| lo.is_none_or(|(x, eq)| v > x || eq && v == x) && hi.is_none_or(|(x, eq)| v < x || eq && v == x);
            let mut want: Vector<u32> = found.ids().filter(|&id| inside(value(id, 1))).collect();
            want.sort_unstable();
            assert_eq!(got, want, "{lo:?} {hi:?}");
        }
    }

    /// A small run merged into a large one by binary searches (`galloped`) is the walking merge's run: the same ids,
    /// tags, prints and fences, for full-row (tagged), prefix and printed orders, the small run on either side.
    #[test]
    fn a_galloped_merge_is_the_walking_merge() {
        let k = 3;
        let rows: Vector<i64> = (0..2100u32).flat_map(|i| [i64::from(i % 37), i64::from((i * 7) % 11), i64::from(i % 500)]).collect();
        for (cols, printed) in [(rerail_blocks::vector![0, 1, 2], false), (rerail_blocks::vector![0], false), (rerail_blocks::vector![1, 0], true), (rerail_blocks::vector![2], false)] {
            let mut ix = if printed { Index::printed(cols.clone()) } else { Index::new(cols.clone()) };
            ix.add_run_in(&rows[..], k, 0, 2000, 1, &[]);
            ix.add_run_in(&rows[..], k, 2000, 2100, 2, &[]);
            let (large, small) = (&ix.runs[0].0, &ix.runs[1].0);
            for (x, y) in [(large, small), (small, large)] {
                let walked = Run::walked(&rows[..], k, &cols, x, y);
                let galloped = if x.ids.len() > y.ids.len() { Run::galloped(&rows[..], k, &cols, x, y) } else { Run::galloped(&rows[..], k, &cols, y, x) };
                assert_eq!(walked.ids.to_vec(), galloped.ids.to_vec(), "{cols:?}");
                assert_eq!((&walked.tags, &walked.prints, &walked.fences), (&galloped.tags, &galloped.prints, &galloped.fences), "{cols:?}");
            }
        }
    }

    /// Rows known sorted (decision 0180) make runs in place that answer as the sorted runs do: whole and partial
    /// knowledge (pieces in place merged with sorted ones), the full-row order tagged and a prefix order implied by an
    /// untagged one, printed orders, probes by `find` and `find_at`, the dedup test, groups, and merges (runs in place
    /// that meet in order stay in place: no ids listed). A tagged full-row range implies nothing for a prefix order.
    #[test]
    fn runs_of_rows_known_sorted_answer_as_sorted_runs() {
        let k = 3;
        let n = 3000u32;
        // Three waves, each written sorted by the full-row order (tagged), as `Fresh::settle` writes them; a fourth
        // sorted by [0, 1] untagged.
        let mut rows: Vector<i64> = rerail_blocks::vector::Vector::new();
        let mut known = Sorted::default();
        let full: Vector<usize> = rerail_blocks::vector![0, 1, 2];
        let waves = [(0u32, 900u32), (900, 2000), (2000, 2600), (2600, n)];
        for (w, &(lo, hi)) in waves.iter().enumerate() {
            let mut wave: Vector<[i64; 3]> = (lo..hi).map(|i| [i64::from(i % 37), i64::from((i * 7) % 11), i64::from(i)]).collect();
            if w < 3 {
                wave.sort_by(|a, b| cmp_vals(&full, true, a, b));
            } else {
                wave.sort_by(|a, b| cmp_vals(&[0, 1], false, a, b).then(a[2].cmp(&b[2])));
            }
            rows.extend(wave.iter().flatten());
            known.note(&rows[..], k, if w < 3 { &full } else { &[0, 1, 2][..2] }, lo, hi);
        }
        assert_eq!(known.rows(), u64::from(n));
        assert!(known.within(&[0], k, 0, n).iter().all(|&(a, _)| a >= 2600), "a tagged range implies no prefix order");
        let ids = |f: Found| { let mut v: Vector<u32> = f.ids().collect(); v.sort_unstable(); v };
        for (cols, printed) in [(rerail_blocks::vector![0, 1, 2], false), (rerail_blocks::vector![0], false), (rerail_blocks::vector![0, 1], true), (rerail_blocks::vector![0], true)] {
            let make = |use_known: bool| {
                let mut ix = if printed { Index::printed(cols.clone()) } else { Index::new(cols.clone()) };
                for (w, &(lo, hi)) in waves.iter().enumerate() {
                    // Partial knowledge too: the second wave's range told only for its first half.
                    let mut kn = if use_known { known.within(&cols, k, lo, hi) } else { Vec::new() };
                    if w == 1 { kn.iter_mut().for_each(|r| r.1 = r.1.min(1450)); }
                    ix.add_run_in(&rows[..], k, lo as usize, hi as usize, w as u32 + 1, &kn);
                }
                ix
            };
            let (plain, placed) = (make(false), make(true));
            for (a, b) in plain.runs.iter().zip(&placed.runs) {
                assert_eq!(a.0.ids.to_vec(), b.0.ids.to_vec(), "{cols:?}: the same order");
                assert_eq!((&a.0.tags, &a.0.prints, &a.0.fences), (&b.0.tags, &b.0.prints, &b.0.fences), "{cols:?}");
            }
            if cols != [0] || printed {
                assert!(placed.bytes() < plain.bytes(), "{cols:?}: runs in place list no ids");
            }
            let check = |ix_a: &Index, ix_b: &Index| {
                let (mut ha, mut hb) = ([0u32; HINTS], [0u32; HINTS]);
                for a in 0..40i64 {
                    for b in [None, Some(3i64), Some(12)] {
                        let key: Vector<i64> = std::iter::once(a).chain(b).take(cols.len()).collect();
                        for delta in [None, Some(2), Some(4)] {
                            assert_eq!(ids(ix_a.find(&rows[..], k, &key, delta)), ids(ix_b.find(&rows[..], k, &key, delta)), "{cols:?} {key:?} {delta:?}");
                        }
                        assert_eq!(ids(ix_a.find_at(&rows[..], k, &key, None, &mut ha)), ids(ix_b.find_at(&rows[..], k, &key, None, &mut hb)), "{cols:?} {key:?}");
                    }
                }
                let groups = |ix: &Index| { let mut g: Vec<Vector<u32>> = Vec::new(); ix.each_group(&rows[..], k, 1, |p| g.push(p.iter().collect())); g };
                assert_eq!(groups(ix_a), groups(ix_b), "{cols:?}: groups");
            };
            check(&plain, &placed);
            if cols.len() == k {
                // The dedup test, over rows held and not.
                let probe: Vector<i64> = (0..200i64).flat_map(|i| [i % 40, (i * 7) % 12, i * 15]).collect();
                let mut order: Vector<u32> = (0..200).collect();
                order.sort_by(|&a, &b| cmp_vals(&cols, true, &probe[a as usize * k..][..k], &probe[b as usize * k..][..k]));
                let (mut ha, mut hb) = (vec![false; 200], vec![false; 200]);
                plain.held_sorted(&rows[..], k, &probe[..], &order, |_| true, &mut ha);
                placed.held_sorted(&rows[..], k, &probe[..], &order, |_| true, &mut hb);
                assert_eq!(ha, hb);
                assert!(ha.iter().any(|&h| h) && ha.iter().any(|&h| !h));
            }
            // Merged (a frozen order's one run): the same rows, and the same answers.
            let (mut pm, mut qm) = (plain.clone(), placed.clone());
            pm.tier(&rows[..], k, 1);
            qm.tier(&rows[..], k, 1);
            assert_eq!(pm.runs[0].0.ids.to_vec(), qm.runs[0].0.ids.to_vec(), "{cols:?}: merged");
            check(&pm, &qm);
        }
        // Runs in place that meet in order merge in place.
        let mut one = Index::new(rerail_blocks::vector![0, 1]);
        one.add_run_in(&rows[..], k, 2600, 2800, 1, &[(2600, 2800)]);
        one.add_run_in(&rows[..], k, 2800, n as usize, 1, &[(2800, n)]);
        one.tier(&rows[..], k, 1);
        assert!(matches!(one.runs[0].0.ids, Ids::Rows(2600, 3000)), "joined in place");
    }

    /// Pieces known sorted, merged with the sorted stretches between them (`sort_pieces`), give the full sort's
    /// order: no pieces, one piece covering all, pieces at either end, many short ones, ties between pieces (kept
    /// left first), and stretches between them in any order.
    #[test]
    fn pieces_known_sorted_merge_into_the_sort() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move |m: u64| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x % m };
        for case in 0..40 {
            let n = [0usize, 1, 7, 300, 2000][case % 5];
            let vals: Vector<u64> = (0..n).map(|_| rand(50)).collect();
            // Pieces: ranges sorted in place in `v` (positions), with unsorted stretches between.
            let mut v: Vector<u32> = (0..n as u32).collect();
            let mut known = Vec::new();
            let mut at = 0;
            while at < n {
                let len = (1 + rand(if case % 3 == 0 { 400 } else { 40 }) as usize).min(n - at);
                if rand(3) > 0 {
                    v[at..at + len].sort_by_key(|&i| vals[i as usize]);
                    known.push((at, at + len));
                }
                at += len;
            }
            if case == 4 {
                v.sort_by_key(|&i| vals[i as usize]);
                known = vec![(0, n)];
            }
            let mut want = v.clone();
            want.sort_by_key(|&i| vals[i as usize]);
            let taken = sort_pieces(&mut v, &known, |s| s.sort_by_key(|&i| vals[i as usize]), |a, b| vals[a as usize] <= vals[b as usize]);
            assert_eq!(v.iter().map(|&i| vals[i as usize]).collect::<Vec<_>>(), want.iter().map(|&i| vals[i as usize]).collect::<Vec<_>>(), "case {case}");
            let mut same = v.clone();
            same.sort_unstable();
            assert_eq!(same, (0..n as u32).collect::<Vec<_>>(), "case {case}: each item once");
            assert_eq!(taken, known.iter().map(|&(a, b)| b - a).sum::<usize>());
        }
    }

    /// Packed keys (decision 0186) sort as the 24-byte tuples do, ids and tags alike: values narrow (a u64), wide or
    /// negative (a u128), node ids (`d << 32 | x`), and too wide to pack (the tuples); one, two and three columns,
    /// tagged (every column) and not; keys equal in their first two values (left to `cmp`) and repeated rows (by id).
    #[test]
    fn packed_keys_sort_as_the_tuples_do() {
        let k = 3;
        let n = 3000i64;
        let shapes: [(&str, fn(i64) -> i64); 5] = [
            ("narrow", |i| i % 37),
            ("negative", |i| (i % 211) - 105),
            ("node", |i| ((i % 9) << 32) | ((i * 7) % 4001)),
            ("wide", |i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64)),
            ("extremes", |i| [i64::MIN, -1, 0, 1, i64::MAX][i as usize % 5]),
        ];
        let mut packed_seen = [false; 2];
        for (name, f) in shapes {
            let rows: Vector<i64> = (0..n).flat_map(|i| [f(i), f(i / 3), i % 5]).collect();
            for (cols, tagged) in [(rerail_blocks::vector![0], false), (rerail_blocks::vector![0, 1], false), (rerail_blocks::vector![1, 0], false), (rerail_blocks::vector![0, 1, 2], true)] {
                let val = |id: u32, i: usize| rows[id as usize * k + cols[i]];
                let cmp = |a: u32, b: u32| cmp_rows(&rows[..], k, &cols, a, b);
                let start: Vector<u32> = (0..n as u32).rev().collect();
                let (mut a, mut b) = (start.clone(), start);
                let (mut ta, mut tb) = (Vec::new(), Vec::new());
                let packed = sort_packed(&mut a, tagged, cols.len(), &val, &cmp, &mut ta);
                sort_tupled(&mut b, tagged, cols.len(), val, cmp, &mut tb);
                packed_seen[usize::from(name == "wide")] |= packed;
                if !packed { continue }
                assert_eq!(a, b, "{name} {cols:?} tagged={tagged}");
                assert_eq!(ta, tb, "{name} {cols:?} tagged={tagged}");
            }
        }
        assert!(packed_seen[0], "the narrow shapes pack");
    }

    /// A tagged order whose leading values all have one tag: the tag's field is zero-width, and its place, past the
    /// values and the ids, is the key's whole width when they fill all 128 bits. Each row's tag is then that tag, as
    /// the tuples give it; shifted by the key's width it was an overflow, and a release build (the shift taken
    /// modulo 128, by 0) gave the key's low bits as the tag.
    #[test]
    fn a_tag_every_row_shares_is_each_rows_tag_when_the_values_fill_the_key() {
        // Values whose hashes share their high 16 bits (tag 0xBE36), spanning 62 bits, beside values spanning 64, and
        // ids of 2 bits: 0 + 62 + 64 + 2 = 128 bits.
        let lead = [-9_217_541_510_037_437_257, -9_204_588_067_223_325_516, -9_196_900_843_560_476_845, -6_910_328_919_769_916_054i64];
        let rows: Vec<i64> = lead.iter().zip([i64::MIN, i64::MAX, 0, -1]).flat_map(|(&a, b)| [a, b]).collect();
        let cols = [0, 1];
        let val = |id: u32, i: usize| rows[id as usize * 2 + cols[i]];
        let cmp = |a: u32, b: u32| cmp_rows(&rows[..], 2, &cols, a, b);
        let (mut a, mut b) = (vec![3, 2, 1, 0], vec![3, 2, 1, 0]);
        let (mut ta, mut tb) = (Vec::new(), Vec::new());
        assert!(sort_packed(&mut a, true, 2, &val, &cmp, &mut ta), "128 bits pack");
        sort_tupled(&mut b, true, 2, val, cmp, &mut tb);
        assert_eq!(tb, [0xBE36; 4]);
        assert_eq!((a, ta), (b, tb));
    }

    /// Measurement aid (decision 0186): instructions and cycles a row of the packed and tupled sorts, over orders of
    /// one and two columns shaped as the O125 replay's large derived-input builds (node ids, `file << 32 | at`).
    #[test]
    #[ignore]
    fn sort_keyed_costs() {
        let n = 50_000u32;
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        for shape in ["clustered", "shuffled", "small"] {
            let rows: Vector<i64> = (0..n as i64).flat_map(|i| {
                let node = match shape { "clustered" => ((i / 600) << 32) | ((i % 600) * 3), "shuffled" => (((rand() % 9000) as i64) << 32) | (rand() % 4000) as i64, _ => (rand() % 40_000) as i64 };
                [node, (rand() % 5000) as i64]
            }).collect();
            for cols in [vec![0usize], vec![0, 1]] {
                let k = 2;
                let val = |id: u32, i: usize| rows[id as usize * k + cols[i]];
                let cmp = |a: u32, b: u32| cmp_rows(&rows[..], k, &cols, a, b);
                let cost = |packed: bool| {
                    let (mut ins, mut cyc) = (u64::MAX, u64::MAX);
                    for _ in 0..5 {
                        let mut ids: Vector<u32> = (0..n).collect();
                        let (i0, c0) = rerail_compiler::native_cpu_codegen::retired();
                        if packed { std::hint::black_box(sort_packed(&mut ids, false, cols.len(), &val, &cmp, &mut NoTags)); } else { sort_tupled(&mut ids, false, cols.len(), val, cmp, &mut NoTags); std::hint::black_box(&ids); }
                        let (i1, c1) = rerail_compiler::native_cpu_codegen::retired();
                        ins = ins.min(i1 - i0);
                        cyc = cyc.min(c1 - c0);
                    }
                    (ins as f64 / f64::from(n), cyc as f64 / f64::from(n))
                };
                let (t, p) = (cost(false), cost(true));
                println!("sort {shape} cols {cols:?}: tupled {:.0} ins {:.0} cyc a row; packed {:.0} ins {:.0} cyc a row", t.0, t.1, p.0, p.1);
            }
        }
    }

    /// A printed order (a derived input's own index) finds what the plain order finds, key for key: whole keys
    /// present and absent, by `find` and by `find_at` (a sorted batch and a shuffled one), over several runs and after
    /// they merge; with one leading value spanning many fences, a tagged order (every column), and prints equal by
    /// chance (thousands of keys over 256 prints).
    #[test]
    fn a_printed_order_finds_what_the_rows_find() {
        let k = 3;
        // Half the rows lead with 0..3 (each value over many fences), half with a wide range; keys repeat.
        let n = 6000i64;
        let rows: Vector<i64> = (0..n).flat_map(|i| [if i % 2 == 0 { i % 3 } else { (i * 7919) % 2003 }, (i * 31) % 97, i % 1009]).collect();
        let ids = |f: Found| { let mut v: Vector<u32> = f.ids().collect(); v.sort_unstable(); v };
        for cols in [rerail_blocks::vector![0], rerail_blocks::vector![0, 1], rerail_blocks::vector![0, 1, 2]] {
            let (mut plain, mut printed) = (Index::new(cols.clone()), Index::printed(cols.clone()));
            for (w, (lo, hi)) in [(0, 2500), (2500, 4000), (4000, 6000)].into_iter().enumerate() {
                plain.add_run(&rows, k, lo, hi, w as u32 + 1);
                printed.add_run(&rows, k, lo, hi, w as u32 + 1);
            }
            // Keys: every row's, and as many absent (values no row holds in some column).
            let mut keys: Vec<Vector<i64>> = (0..n as usize).map(|r| cols.iter().map(|&c| rows[r * k + c]).collect()).collect();
            keys.extend((0..n).map(|i| cols.iter().enumerate().map(|(j, _)| if j + 1 == cols.len() { 5000 + i } else { i % 3 }).collect()));
            let check = |plain: &Index, printed: &Index, keys: &[Vector<i64>]| {
                let (mut h0, mut h1) = ([0u32; HINTS], [0u32; HINTS]);
                for key in keys {
                    let want = ids(plain.find(&rows, k, key, None));
                    assert_eq!(ids(printed.find(&rows, k, key, None)), want, "{cols:?} {key:?}");
                    assert_eq!(ids(printed.find_at(&rows, k, key, None, &mut h1)), ids(plain.find_at(&rows, k, key, None, &mut h0)), "{cols:?} {key:?} (find_at)");
                    assert_eq!(ids(printed.find(&rows, k, key, Some(2))), ids(plain.find(&rows, k, key, Some(2))), "{cols:?} {key:?} (delta)");
                }
            };
            // Shuffled, then sorted as the order is (a sorted batch of probes).
            let mut shuffled = keys.clone();
            shuffled.sort_by_key(|key| key.iter().fold(0u64, |h, &v| (h ^ v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
            check(&plain, &printed, &shuffled);
            let tagged = tagged(&cols, k);
            keys.sort_by(|a, b| cmp_vals(&(0..cols.len()).collect::<Vec<_>>(), tagged, a, b));
            check(&plain, &printed, &keys);
            // Merged: prints carried through, the same finds.
            plain.tier(&rows, k, 1);
            printed.tier(&rows, k, 1);
            assert_eq!(printed.runs(), 1);
            check(&plain, &printed, &shuffled);
            // A byte a row more, its chunk kept to an eighth of its class past it (`Block::try_alloc_chunk_fit`).
            let more = printed.bytes() - plain.bytes();
            assert!(more >= printed.rows() && more <= printed.rows() * 9 / 8 + 64, "a byte a row more: {more} for {}", printed.rows());
        }
    }

    /// The order on every column is tagged (decision 0112, the dedup test sorted by 16 bits of the leading value's
    /// hash first): keys are found by their tags, long spans of one leading value included, and every prefix is still
    /// a range.
    #[test]
    fn a_tagged_order_finds_keys_in_long_spans_of_one_leading_value() {
        let k = 3;
        let rows: Vector<i64> = (0..20_000i64).flat_map(|i| [i % 3, (i * 7919) % 500, i]).collect();
        let mut ix = Index::new(rerail_blocks::vector![0, 1, 2]);
        ix.add_run(&rows, k, 0, 12_000, 1);
        ix.add_run(&rows, k, 12_000, 20_000, 2);
        assert!(ix.runs.iter().all(|(r, _)| r.tags.len() == r.ids.len()), "the full-row order keeps a tag an id");
        let want = |a: i64, b: Option<i64>| (0..20_000i64).filter(|i| i % 3 == a && b.is_none_or(|b| (i * 7919) % 500 == b)).count();
        for a in 0..4 {
            assert_eq!(ix.find(&rows, k, &[a], None).len(), want(a, None));
            for b in [0, 7, 251, 499, 500] {
                assert_eq!(ix.find(&rows, k, &[a, b], None).len(), want(a, Some(b)), "({a}, {b})");
            }
        }
        assert_eq!(ix.find(&rows, k, &[1, (4 * 7919) % 500, 4], None).len(), 1);
        assert_eq!(ix.find(&rows, k, &[1, (4 * 7919) % 500, 5], None).len(), 0);
        // The dedup test, a sorted batch: rows held and not, sorted as the order sorts them.
        let probe: Vector<i64> = [[1, (4 * 7919) % 500, 4], [1, (4 * 7919) % 500, 5], [2, (5 * 7919) % 500, 5], [0, 3, 20_001]].concat().into_iter().collect();
        let mut order: Vector<u32> = (0..4).collect();
        order.sort_by(|&a, &b| cmp_vals(&ix.cols, true, &probe[a as usize * k..][..k], &probe[b as usize * k..][..k]));
        let mut held = vec![false; 4];
        ix.held_sorted(&rows, k, &probe[..], &order, |_| true, &mut held);
        let by: Vec<bool> = (0..4u32).map(|q| held[order.iter().position(|&o| o == q).unwrap()]).collect();
        assert_eq!(by, [true, false, true, false]);
        let (mut groups, mut seen) = (0, 0);
        ix.each_group(&rows, k, 2, |ids| {
            groups += 1;
            seen += ids.len();
        });
        assert_eq!((groups, seen), (1500, 20_000), "each (a, b) one group");
    }

    /// A tagged order sorted by its tags as integers, then rows only within a tag, is the comparator's sort; a merge
    /// keeps each id's tag beside it.
    #[test]
    fn sorting_by_tag_is_the_orders_sort() {
        let k = 3;
        let rows: Vector<i64> = (0..30_000i64).flat_map(|i| [(i * 37) % 1009, i % 5, (i * 7) % 11]).collect();
        let cols = [0usize, 1, 2];
        let run = Run::of(&rows, k, &cols, 0..30_000u32);
        let mut want: Vector<u32> = (0..30_000).collect();
        want.sort_unstable_by(|&a, &b| cmp_rows(&rows, k, &cols, a, b));
        assert_eq!(run.ids.to_vec(), want);
        assert!(run.ids.iter().zip(&run.tags).all(|(id, &t)| t == tag(rows[id as usize * k])));
        let (a, b) = (Run::of(&rows, k, &cols, 0..12_000u32), Run::of(&rows, k, &cols, 12_000..30_000u32));
        let merged = Run::merge(&rows, k, &cols, &a, &b);
        assert_eq!(merged.ids.to_vec(), want);
        assert_eq!(merged.tags, run.tags);
    }

    /// A tag counted at each comparison (`tag_span` reads tags only by comparing them); only
    /// `a_tag_span_costs_reads_logarithmic_in_its_length` makes them, so one counter serves.
    #[derive(Clone, Copy, Debug)]
    struct Read(u16);
    static READS: AtomicU64 = AtomicU64::new(0);
    impl PartialEq for Read {
        fn eq(&self, o: &Read) -> bool {
            READS.fetch_add(1, AtomicOrdering::Relaxed);
            self.0 == o.0
        }
    }
    impl Eq for Read {}
    impl PartialOrd for Read {
        fn partial_cmp(&self, o: &Read) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Read {
        fn cmp(&self, o: &Read) -> Ordering {
            READS.fetch_add(1, AtomicOrdering::Relaxed);
            self.0.cmp(&o.0)
        }
    }
    impl From<Read> for u64 {
        fn from(t: Read) -> u64 {
            t.0 as u64
        }
    }

    /// A span of one tag is found in reads logarithmic in its length, from any place in it: `held_sorted` searches a
    /// sorted batch's rows from where the last one ended, and a batch of rows sharing a leading value (so a tag) read
    /// the rest of the span for each row when its end was walked: 1,000 rows over a span of 2^20 read 524M tags.
    #[test]
    fn a_tag_span_costs_reads_logarithmic_in_its_length() {
        let span = 1usize << 20;
        let tags: Vec<Read> = std::iter::repeat_n(Read(1), 10).chain(std::iter::repeat_n(Read(5), span)).chain(std::iter::repeat_n(Read(9), 10)).collect();
        let rows = 1000;
        READS.store(0, AtomicOrdering::Relaxed);
        for i in 0..rows {
            let from = 10 + i * (span / rows);
            assert_eq!(tag_span(&tags, Read(5), from, false), (from, 10 + span));
        }
        assert_eq!(tag_span(&tags, Read(5), 0, true), (10, 10 + span));
        assert_eq!(tag_span(&tags, Read(9), 0, true), (10 + span, 20 + span));
        let reads = READS.load(AtomicOrdering::Relaxed);
        // About 4 log2(2^20) reads a search; the walk read about rows * span / 2.
        assert!(reads <= (rows as u64 + 2) * 100, "{reads} tag reads for {rows} searches of a span of {span}");
    }
}
