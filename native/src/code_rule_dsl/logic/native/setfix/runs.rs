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

use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

/// Ids between fences.
const FENCE: usize = 64;

/// A probe over a run costs about this many times the merge's cost of an id (O125: ~40 ns a run's probe, ~5 ns an
/// id merged).
const PROBE_PER_MERGED_ID: u64 = 8;

/// One run: row ids sorted by the order's columns (then id), and the leading value of every `FENCE`th id. A tagged
/// run (the order on every column, of two or more: the dedup test) sorts by 16 bits of the leading value's hash
/// (`tag`) first, then the columns in turn, and keeps each id's tag (`tags`) instead of fences: a key's ids are found
/// by its tag among the tags (placed by interpolation, as hash order spreads them evenly), and only the ids of that
/// tag are read. Equal leading values have equal tags, so every prefix of the columns is still a range.
pub(super) struct Run {
    ids: Vec<u32>,
    fences: Vec<i64>,
    tags: Vec<u16>,
}

/// 16 bits of a value's hash (Fibonacci hashing: the product's high bits).
#[inline]
pub(super) fn tag(v: i64) -> u16 {
    ((v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) as u16
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
    let Some(&lead) = cols.first() else { return Ordering::Equal };
    (if tagged { tag(ra[lead]).cmp(&tag(rb[lead])) } else { Ordering::Equal })
        .then_with(|| cols.iter().map(|&c| ra[c].cmp(&rb[c])).find(|o| o.is_ne()).unwrap_or(Ordering::Equal))
}

/// The span of `tags` (sorted) equal to `t`, searched from `from`: a galloping search forward from there, or (with
/// `place`) from where an even spread of tags would put it.
#[inline]
fn tag_span(tags: &[u16], t: u16, from: usize, place: bool) -> (usize, usize) {
    let n = tags.len();
    if from >= n {
        return (n, n);
    }
    let mut at = if place { from.max(((t as usize) * n) >> 16) } else { from };
    // Back while the place is past the span's start (it can be, placed by interpolation).
    let mut step = 1;
    while at > from && tags[at.min(n - 1)] >= t {
        at = at.saturating_sub(step).max(from);
        step *= 2;
    }
    // Forward, galloping, to the first tag at or past `t`.
    let mut bound = 1;
    while at + bound <= n && tags[at + bound - 1] < t {
        bound *= 2;
    }
    let lo = if bound > 1 { at + bound / 2 } else { at };
    let hi = (at + bound - 1).min(n);
    let s = lo + tags[lo..hi].partition_point(|&x| x < t);
    let mut e = s;
    while e < n && tags[e] == t {
        e += 1;
    }
    (s, e)
}

/// An order on the columns `cols` (every key set it answers is a prefix of them).
pub(super) struct Index {
    pub(super) cols: Vec<usize>,
    /// The runs, oldest first, each with the wave whose rows it holds (0: of no wave's delta).
    runs: Vec<(Arc<Run>, u32)>,
    /// Runs read past the first by probes since the last merge.
    rent: AtomicU64,
}

impl Clone for Index {
    fn clone(&self) -> Index {
        Index { cols: self.cols.clone(), runs: self.runs.clone(), rent: AtomicU64::new(0) }
    }
}

/// Rows `a` and `b` in the order on `cols` (`cmp_vals`), then by id (so a sort is the same whatever order it came in).
#[inline]
fn cmp_rows(data: &[i64], k: usize, cols: &[usize], a: u32, b: u32) -> Ordering {
    let (ra, rb) = (&data[a as usize * k..][..k], &data[b as usize * k..][..k]);
    cmp_vals(cols, tagged(cols, k), ra, rb).then_with(|| a.cmp(&b))
}

impl Run {
    fn fenced(data: &[i64], k: usize, cols: &[usize], ids: Vec<u32>) -> Run {
        if tagged(cols, k) {
            let tags = ids.iter().map(|&id| tag(data[id as usize * k + cols[0]])).collect();
            return Run { ids, fences: Vec::new(), tags };
        }
        let fences = match cols.first() {
            Some(&lead) => ids.iter().step_by(FENCE).map(|&id| data[id as usize * k + lead]).collect(),
            None => Vec::new(),
        };
        Run { ids, fences, tags: Vec::new() }
    }

    /// The ids of `ids` sorted by `cols` (then id).
    fn of(data: &[i64], k: usize, cols: &[usize], mut ids: Vec<u32>) -> Run {
        if tagged(cols, k) {
            let lead = cols[0];
            let tags = sort_by_tag(&mut ids, |id| tag(data[id as usize * k + lead]), |a, b| cmp_rows(data, k, cols, a, b));
            return Run { ids, fences: Vec::new(), tags };
        }
        ids.sort_unstable_by(|&a, &b| cmp_rows(data, k, cols, a, b));
        Run::fenced(data, k, cols, ids)
    }

    /// The ids whose first `key.len()` columns of `cols` hold `key`.
    #[inline]
    fn find(&self, data: &[i64], k: usize, cols: &[usize], key: &[i64]) -> &[u32] {
        // No columns: every row (a relation of no columns holds one row at most).
        let Some(&lead) = key.first() else { return &self.ids };
        let cmp = |id: u32| -> Ordering {
            let r = &data[id as usize * k..][..k];
            cols.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let window = if !self.tags.is_empty() {
            // The leading value's tag among the tags: only its ids are read.
            let (s, e) = tag_span(&self.tags, tag(lead), 0, true);
            &self.ids[s..e]
        } else {
            // The rows of `lead` start in the block before the first fence at or past it, and end before the first
            // fence past it.
            let a = self.fences.partition_point(|&f| f < lead);
            let b = a + self.fences[a..].partition_point(|&f| f == lead);
            &self.ids[a.saturating_sub(1) * FENCE..(b * FENCE).min(self.ids.len())]
        };
        let s = window.partition_point(|&id| cmp(id) == Ordering::Less);
        let e = s + window[s..].partition_point(|&id| cmp(id) == Ordering::Equal);
        &window[s..e]
    }

    /// `find`, from where the last key of a sequence was found (`hint`, a position in this run, set past this key's
    /// ids): the tags or fences are searched galloping from there, either way, so a key near the last one (a sorted
    /// batch of keys, decision 0112) reads only what is near, and a far one costs about a search from scratch.
    #[inline]
    fn find_at(&self, data: &[i64], k: usize, cols: &[usize], key: &[i64], hint: &mut u32) -> &[u32] {
        let Some(&lead) = key.first() else { return &self.ids };
        let n = self.ids.len();
        let cmp = |id: u32| -> Ordering {
            let r = &data[id as usize * k..][..k];
            cols.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let (lo, hi) = if !self.tags.is_empty() {
            let t = tag(lead);
            let s = gallop(&self.tags, (*hint as usize).min(n), |&x| x < t);
            (s, s + gallop(&self.tags[s..], 0, |&x| x == t))
        } else {
            let a = gallop(&self.fences, (*hint as usize / FENCE).min(self.fences.len()), |&f| f < lead);
            let b = a + gallop(&self.fences[a..], 0, |&f| f == lead);
            (a.saturating_sub(1) * FENCE, (b * FENCE).min(n))
        };
        let window = &self.ids[lo..hi];
        let s = window.partition_point(|&id| cmp(id) == Ordering::Less);
        let e = s + window[s..].partition_point(|&id| cmp(id) == Ordering::Equal);
        *hint = (lo + e) as u32;
        &window[s..e]
    }

    /// Sets `held[i]` when a live id (`live`) holds row `order[i]` of `rows` (flat, `k` columns each): `order` lists
    /// distinct rows sorted as this run is (`cmp_vals`), so each row's search starts where the last one's ended. Its
    /// block is found by the fences (leading values, inline), or in a tagged run its tag's span by the tags; within it
    /// the search gallops, so rows near each other in the order read few rows of `data`.
    fn held_sorted(&self, data: &[i64], k: usize, cols: &[usize], rows: &[i64], order: &[u32], live: &impl Fn(u32) -> bool, held: &mut [bool]) {
        let n = self.ids.len();
        let Some(&lead) = cols.first() else {
            // No columns: one row at most, held when a live id is.
            if self.ids.iter().any(|&id| live(id)) {
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
            let row = &rows[q as usize * k..][..k];
            let cmp = |id: u32| -> Ordering { cmp_vals(cols, tagged, &data[id as usize * k..][..k], row) };
            if tagged {
                // The row's tag span, from where the last row's search ended; within it, the rows read in order.
                let (s, e) = tag_span(&self.tags, tag(row[lead]), at, false);
                let p = s + self.ids[s..e].partition_point(|&id| cmp(id).is_lt());
                at = p;
                held[i] = self.ids[p..e].iter().take_while(|&&id| cmp(id).is_eq()).any(|&id| live(id));
                continue;
            }
            // The first id at or past the row is past the block before the first fence at or past its leading value.
            let v = row[lead];
            let from = at / FENCE;
            let f = from + self.fences[from..].partition_point(|&x| x < v);
            let start = at.max(f.saturating_sub(1) * FENCE);
            let mut bound = 1;
            while start + bound <= n && cmp(self.ids[start + bound - 1]).is_lt() {
                bound *= 2;
            }
            let lo = if bound > 1 { start + bound / 2 } else { start };
            let hi = (start + bound - 1).min(n);
            let p = lo + self.ids[lo..hi].partition_point(|&id| cmp(id).is_lt());
            at = p;
            held[i] = self.ids[p..].iter().take_while(|&&id| cmp(id).is_eq()).any(|&id| live(id));
        }
    }

    /// Two runs as one.
    fn merge(data: &[i64], k: usize, cols: &[usize], a: &Run, b: &Run) -> Run {
        let mut ids = Vec::with_capacity(a.ids.len() + b.ids.len());
        let tagged = !a.tags.is_empty() && !b.tags.is_empty();
        let mut tags = Vec::with_capacity(if tagged { ids.capacity() } else { 0 });
        let (mut i, mut j) = (0, 0);
        while i < a.ids.len() && j < b.ids.len() {
            // Tagged, the tags decide unless equal: rows are read only for ids of one tag.
            let first = if tagged && a.tags[i] != b.tags[j] { a.tags[i] < b.tags[j] } else { cmp_rows(data, k, cols, a.ids[i], b.ids[j]).is_le() };
            if first {
                ids.push(a.ids[i]);
                if tagged { tags.push(a.tags[i]); }
                i += 1;
            } else {
                ids.push(b.ids[j]);
                if tagged { tags.push(b.tags[j]); }
                j += 1;
            }
        }
        ids.extend_from_slice(&a.ids[i..]);
        ids.extend_from_slice(&b.ids[j..]);
        if tagged {
            tags.extend_from_slice(&a.tags[i..]);
            tags.extend_from_slice(&b.tags[j..]);
            return Run { ids, fences: Vec::new(), tags };
        }
        Run::fenced(data, k, cols, ids)
    }

    /// Calls `f` on each group of ids equal on the first `m` columns of `cols`, in order.
    fn each_group(&self, data: &[i64], k: usize, cols: &[usize], m: usize, mut f: impl FnMut(&[u32])) {
        let same = |a: u32, b: u32| cols[..m].iter().all(|&c| data[a as usize * k + c] == data[b as usize * k + c]);
        let mut at = 0;
        while at < self.ids.len() {
            let mut end = at + 1;
            while end < self.ids.len() && same(self.ids[at], self.ids[end]) {
                end += 1;
            }
            f(&self.ids[at..end]);
            at = end;
        }
    }
}

/// A key's candidate ids: its range of each run that holds it (the first few inline, so a probe allocates nothing).
#[derive(Clone, Default)]
pub(super) struct Found<'a> {
    first: [&'a [u32]; 3],
    n: usize,
    rest: Vec<&'a [u32]>,
    len: usize,
}

impl<'a> Found<'a> {
    fn add(&mut self, s: &'a [u32]) {
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
    pub(super) fn parts(&self) -> impl Iterator<Item = &'a [u32]> + '_ {
        self.first[..self.n.min(self.first.len())].iter().copied().chain(self.rest.iter().copied())
    }
}

/// The first position of `v` (partitioned: `less` holds before it, not from it) not `less`: a galloping search from
/// `start`, forward or back, so its cost grows with the distance from `start`, not with `v`'s length.
#[inline]
fn gallop<T>(v: &[T], start: usize, less: impl Fn(&T) -> bool) -> usize {
    let n = v.len();
    let start = start.min(n);
    if start < n && less(&v[start]) {
        // Forward: the first not `less` is past `start`.
        let mut step = 1;
        let mut lo = start + 1;
        while lo + step <= n && less(&v[lo + step - 1]) {
            lo += step;
            step *= 2;
        }
        let hi = (lo + step).min(n);
        lo + v[lo..hi].partition_point(|x| less(x))
    } else {
        // Back: the first not `less` is at or before `start`.
        let mut step = 1;
        let mut hi = start;
        while hi >= step && !less(&v[hi - step]) {
            hi -= step;
            step *= 2;
        }
        let lo = hi.saturating_sub(step);
        lo + v[lo..hi].partition_point(|x| less(x))
    }
}

/// Runs an order's probes keep their place in (`Index::find_at`); past them, a run is searched from scratch.
pub(super) const HINTS: usize = 8;

/// `ids` sorted by `cmp` where `cmp` orders by `tag_of` first (a tagged order): the ids sorted by (tag, id) as plain
/// integers, then each stretch of one tag by `cmp`, so rows are read only to order ids of equal tags. The sorted
/// ids' tags are returned.
pub(super) fn sort_by_tag(ids: &mut [u32], tag_of: impl Fn(u32) -> u16, cmp: impl Fn(u32, u32) -> Ordering) -> Vec<u16> {
    let mut keys: Vec<u64> = ids.iter().map(|&id| (u64::from(tag_of(id)) << 32) | u64::from(id)).collect();
    keys.sort_unstable();
    let mut tags = Vec::with_capacity(keys.len());
    for (slot, key) in ids.iter_mut().zip(&keys) {
        *slot = *key as u32;
        tags.push((key >> 32) as u16);
    }
    let mut at = 0;
    while at < ids.len() {
        let mut end = at + 1;
        while end < ids.len() && tags[end] == tags[at] {
            end += 1;
        }
        if end - at > 1 {
            ids[at..end].sort_unstable_by(|&a, &b| cmp(a, b));
        }
        at = end;
    }
    tags
}

impl Index {
    pub(super) fn new(cols: Vec<usize>) -> Index {
        Index { cols, runs: Vec::new(), rent: AtomicU64::new(0) }
    }

    /// Whether the set `keys` is a prefix of this order's columns (in any order).
    pub(super) fn answers(&self, keys: &[usize]) -> bool {
        !keys.is_empty() && keys.len() <= self.cols.len() && keys.iter().all(|c| self.cols[..keys.len()].contains(c))
    }

    /// The rows `lo..hi` as a new run of wave `wave` (dead rows are indexed as any; readers skip them).
    pub(super) fn add_run(&mut self, data: &[i64], k: usize, lo: usize, hi: usize, wave: u32) {
        if lo < hi {
            self.runs.push((Arc::new(Run::of(data, k, &self.cols, (lo as u32..hi as u32).collect())), wave));
        }
    }

    /// At a wave's start (before its run is added): merges the runs older than wave `wave` into one when the rent
    /// probes paid since the last merge (runs read past the first) passes what the merge costs.
    pub(super) fn settle(&mut self, data: &[i64], k: usize, wave: u32) {
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
    fn merged_of(&self, data: &[i64], k: usize, n: usize) -> Run {
        // Pairwise, smallest first: each id is copied about log(n) times.
        let mut parts: Vec<Arc<Run>> = self.runs[..n].iter().map(|(r, _)| Arc::clone(r)).collect();
        while parts.len() > 1 {
            parts.sort_by_key(|r| std::cmp::Reverse(r.ids.len()));
            let a = parts.pop().expect("a run");
            let b = parts.pop().expect("a run");
            parts.push(Arc::new(Run::merge(data, k, &self.cols, &a, &b)));
        }
        Arc::try_unwrap(parts.pop().expect("a run")).unwrap_or_else(|r| Run { ids: r.ids.clone(), fences: r.fences.clone(), tags: r.tags.clone() })
    }

    /// At most `max` runs, the two smallest merged at a time (a relation's frozen order: its probes cannot merge it
    /// later, and a large run is left alone while the small ones grow to its size).
    pub(super) fn tier(&mut self, data: &[i64], k: usize, max: usize) {
        while self.runs.len() > max.max(1) {
            let mut by: Vec<usize> = (0..self.runs.len()).collect();
            by.sort_by_key(|&i| self.runs[i].0.ids.len());
            let (a, b) = (by[0].min(by[1]), by[0].max(by[1]));
            let merged = Run::merge(data, k, &self.cols, &self.runs[a].0, &self.runs[b].0);
            self.runs.remove(b);
            self.runs[a] = (Arc::new(merged), 0);
        }
    }

    /// The same rows ordered by `cols` instead, each run re-sorted (its wave kept).
    pub(super) fn reorder(&mut self, data: &[i64], k: usize, cols: Vec<usize>) {
        if cols == self.cols {
            return;
        }
        self.cols = cols;
        for (r, _) in &mut self.runs {
            *r = Arc::new(Run::of(data, k, &self.cols, r.ids.clone()));
        }
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
    pub(super) fn find<'a>(&'a self, data: &[i64], k: usize, key: &[i64], delta: Option<u32>) -> Found<'a> {
        let mut found = Found::default();
        let mut read = 0u64;
        for (run, w) in &self.runs {
            if delta.is_some_and(|d| d != *w) {
                continue;
            }
            read += 1;
            let s = run.find(data, k, &self.cols, key);
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
    pub(super) fn find_at<'a>(&'a self, data: &[i64], k: usize, key: &[i64], delta: Option<u32>, hints: &mut [u32; HINTS]) -> Found<'a> {
        let mut found = Found::default();
        let mut read = 0u64;
        for (r, (run, w)) in self.runs.iter().enumerate() {
            if delta.is_some_and(|d| d != *w) {
                continue;
            }
            read += 1;
            let s = if r < HINTS { run.find_at(data, k, &self.cols, key, &mut hints[r]) } else { run.find(data, k, &self.cols, key) };
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
    pub(super) fn held_sorted(&self, data: &[i64], k: usize, rows: &[i64], order: &[u32], live: impl Fn(u32) -> bool, held: &mut [bool]) {
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
    pub(super) fn each_group(&self, data: &[i64], k: usize, m: usize, f: impl FnMut(&[u32])) {
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

    /// Bytes held: an id a row, its tag (a tagged order), its fences, and each run's own allocation (shared runs
    /// counted by each holder).
    pub(super) fn bytes(&self) -> usize {
        self.runs.iter().map(|(r, _)| r.ids.capacity() * 4 + r.tags.capacity() * 2 + r.fences.capacity() * 8 + 64).sum::<usize>() + self.runs.capacity() * 16
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn orders_find_prefixes_merge_and_delta() {
        let k = 3;
        // Rows (i % 10, i % 7, i) for i in 0..2000: three waves of rows.
        let rows: Vec<i64> = (0..2000i64).flat_map(|i| [i % 10, i % 7, i]).collect();
        let mut by = Index::new(vec![0, 1, 2]);
        let mut by10 = Index::new(vec![1, 0, 2]);
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

    /// The order on every column is tagged (decision 0112, the dedup test sorted by 16 bits of the leading value's
    /// hash first): keys are found by their tags, long spans of one leading value included, and every prefix is still
    /// a range.
    #[test]
    fn a_tagged_order_finds_keys_in_long_spans_of_one_leading_value() {
        let k = 3;
        let rows: Vec<i64> = (0..20_000i64).flat_map(|i| [i % 3, (i * 7919) % 500, i]).collect();
        let mut ix = Index::new(vec![0, 1, 2]);
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
        let probe: Vec<i64> = [[1, (4 * 7919) % 500, 4], [1, (4 * 7919) % 500, 5], [2, (5 * 7919) % 500, 5], [0, 3, 20_001]].concat();
        let mut order: Vec<u32> = (0..4).collect();
        order.sort_by(|&a, &b| cmp_vals(&ix.cols, true, &probe[a as usize * k..][..k], &probe[b as usize * k..][..k]));
        let mut held = vec![false; 4];
        ix.held_sorted(&rows, k, &probe, &order, |_| true, &mut held);
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
        let rows: Vec<i64> = (0..30_000i64).flat_map(|i| [(i * 37) % 1009, i % 5, (i * 7) % 11]).collect();
        let cols = [0usize, 1, 2];
        let run = Run::of(&rows, k, &cols, (0..30_000u32).collect());
        let mut want: Vec<u32> = (0..30_000).collect();
        want.sort_unstable_by(|&a, &b| cmp_rows(&rows, k, &cols, a, b));
        assert_eq!(run.ids, want);
        assert!(run.ids.iter().zip(&run.tags).all(|(&id, &t)| t == tag(rows[id as usize * k])));
        let (a, b) = (Run::of(&rows, k, &cols, (0..12_000).collect()), Run::of(&rows, k, &cols, (12_000..30_000).collect()));
        let merged = Run::merge(&rows, k, &cols, &a, &b);
        assert_eq!(merged.ids, want);
        assert_eq!(merged.tags, run.tags);
    }
}
