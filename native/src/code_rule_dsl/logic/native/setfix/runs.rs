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

/// One run: row ids sorted by the order's columns (then id), and the leading value of every `FENCE`th id.
pub(super) struct Run {
    ids: Vec<u32>,
    fences: Vec<i64>,
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

/// Rows `a` and `b` compared on `cols`, in order, then by id (so a sort is the same whatever order it came in).
#[inline]
fn cmp_rows(data: &[i64], k: usize, cols: &[usize], a: u32, b: u32) -> Ordering {
    let (ra, rb) = (&data[a as usize * k..][..k], &data[b as usize * k..][..k]);
    cols.iter().map(|&c| ra[c].cmp(&rb[c])).find(|o| o.is_ne()).unwrap_or_else(|| a.cmp(&b))
}

impl Run {
    fn fenced(data: &[i64], k: usize, cols: &[usize], ids: Vec<u32>) -> Run {
        let fences = match cols.first() {
            Some(&lead) => ids.iter().step_by(FENCE).map(|&id| data[id as usize * k + lead]).collect(),
            None => Vec::new(),
        };
        Run { ids, fences }
    }

    /// The ids of `ids` sorted by `cols` (then id).
    fn of(data: &[i64], k: usize, cols: &[usize], mut ids: Vec<u32>) -> Run {
        ids.sort_unstable_by(|&a, &b| cmp_rows(data, k, cols, a, b));
        Run::fenced(data, k, cols, ids)
    }

    /// The ids whose first `key.len()` columns of `cols` hold `key`.
    #[inline]
    fn find(&self, data: &[i64], k: usize, cols: &[usize], key: &[i64]) -> &[u32] {
        // No columns: every row (a relation of no columns holds one row at most).
        let Some(&lead) = key.first() else { return &self.ids };
        // The rows of `lead` start in the block before the first fence at or past it, and end before the first fence
        // past it.
        let a = self.fences.partition_point(|&f| f < lead);
        let b = a + self.fences[a..].partition_point(|&f| f == lead);
        let window = &self.ids[a.saturating_sub(1) * FENCE..(b * FENCE).min(self.ids.len())];
        let cmp = |id: u32| -> Ordering {
            let r = &data[id as usize * k..][..k];
            cols.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let s = window.partition_point(|&id| cmp(id) == Ordering::Less);
        let e = s + window[s..].partition_point(|&id| cmp(id) == Ordering::Equal);
        &window[s..e]
    }

    /// Sets `held[i]` when a live id (`live`) holds row `order[i]` of `rows` (flat, `k` columns each): `order` lists
    /// distinct rows sorted by `cols`, so each row's search starts where the last one's ended. The fences (leading
    /// values, inline) find a row's block from there; within it the search gallops, so rows near each other in the
    /// order read few rows of `data`.
    fn held_sorted(&self, data: &[i64], k: usize, cols: &[usize], rows: &[i64], order: &[u32], live: &impl Fn(u32) -> bool, held: &mut [bool]) {
        let n = self.ids.len();
        let Some(&lead) = cols.first() else {
            // No columns: one row at most, held when a live id is.
            if self.ids.iter().any(|&id| live(id)) {
                held.iter_mut().for_each(|h| *h = true);
            }
            return;
        };
        let mut at = 0;
        for (i, &q) in order.iter().enumerate() {
            if at >= n {
                break;
            }
            if held[i] {
                continue;
            }
            let row = &rows[q as usize * k..][..k];
            let cmp = |id: u32| -> Ordering {
                let r = &data[id as usize * k..][..k];
                cols.iter().map(|&c| r[c].cmp(&row[c])).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
            };
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
        let (mut i, mut j) = (0, 0);
        while i < a.ids.len() && j < b.ids.len() {
            if cmp_rows(data, k, cols, a.ids[i], b.ids[j]).is_le() {
                ids.push(a.ids[i]);
                i += 1;
            } else {
                ids.push(b.ids[j]);
                j += 1;
            }
        }
        ids.extend_from_slice(&a.ids[i..]);
        ids.extend_from_slice(&b.ids[j..]);
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
        Arc::try_unwrap(parts.pop().expect("a run")).unwrap_or_else(|r| Run { ids: r.ids.clone(), fences: r.fences.clone() })
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

    /// Bytes held: an id a row and its fences, and each run's own allocation (shared runs counted by each holder).
    pub(super) fn bytes(&self) -> usize {
        self.runs.iter().map(|(r, _)| r.ids.capacity() * 4 + r.fences.capacity() * 8 + 64).sum::<usize>() + self.runs.capacity() * 16
    }
}

#[cfg(test)]
mod tests {
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
}
