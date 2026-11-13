//! A relation's rows as segments (smoking gun 8, t34): a maintained stratum's relation starts as its kept one less
//! the rows gone, and its rows were the kept relation's copied (`Rel::based`, 1.2 GB in 126 starts of one edit).
//! Here the kept relation's segments are shared (`Arc`), its dead rows marked beside them, and the rows a run pushes
//! go in a tail of its own (ids from the kept relation's count on). When the run's relation is closed (`InRel`), its
//! tail is frozen into one more segment, not copied, so the next run shares it in turn.
//!
//! ```text
//! kept run's rows:   [ segment 0: ids 0..9,000 ]
//! this run's start:  [ segment 0 (shared) ] + tail (ids 9,000..)        // nothing copied
//! closed:            [ segment 0 ] [ segment 1: the tail, moved ]
//! ```
//!
//! Every row lies whole in one segment, so a row is one slice. Segments past `MAX_SEGMENTS` are merged from the
//! newest (the small ones a session's edits add), so a row's segment is found among a few.

use std::sync::Arc;

use super::runs::RowData;

/// The most segments a frozen relation holds; past them its newest are merged.
const MAX_SEGMENTS: usize = 8;

#[derive(Clone, Default)]
pub(super) struct Rows {
    /// Frozen segments, oldest first: each its first row's id and its values.
    segs: Vec<(u32, Arc<Vec<i64>>)>,
    /// The first id the tail holds: every frozen segment's rows come before it.
    from: u32,
    /// The rows pushed since the last freeze.
    tail: Vec<i64>,
}

impl Rows {
    /// Rows held as one flat vector (ids from 0).
    pub(super) fn from_vec(rows: Vec<i64>) -> Rows {
        Rows { segs: Vec::new(), from: 0, tail: rows }
    }

    /// The values held, every segment's and the tail's.
    pub(super) fn len(&self) -> usize {
        self.segs.iter().map(|(_, s)| s.len()).sum::<usize>() + self.tail.len()
    }

    /// The bytes of the values held (the frozen segments counted whole, shared or not).
    pub(super) fn bytes(&self) -> usize {
        self.len() * 8
    }

    /// The bytes allocated: the frozen segments' and the tail's capacity.
    pub(super) fn capacity_bytes(&self) -> usize {
        (self.segs.iter().map(|(_, s)| s.capacity()).sum::<usize>() + self.tail.capacity()) * 8
    }

    pub(super) fn reserve(&mut self, values: usize) {
        self.tail.reserve(values);
    }

    /// Appends one row's values (`k` of them) to the tail.
    #[inline]
    pub(super) fn extend_from_slice(&mut self, row: &[i64]) {
        self.tail.extend_from_slice(row);
    }

    /// The values of rows `lo..hi` (`k` a row), which lie in one segment or in the tail: the rows a wave pushed.
    pub(super) fn range(&self, lo: usize, hi: usize, k: usize) -> &[i64] {
        if lo >= self.from as usize {
            let at = lo - self.from as usize;
            return &self.tail[at * k..(at + hi - lo) * k];
        }
        let i = self.seg_of(lo as u32);
        let (first, s) = &self.segs[i];
        let at = lo - *first as usize;
        &s[at * k..(at + hi - lo) * k]
    }

    /// The values of rows `lo..hi`, a slice per segment they span, in order.
    pub(super) fn each_range(&self, lo: usize, hi: usize, k: usize, mut f: impl FnMut(&[i64])) {
        let mut at = lo;
        while at < hi {
            let end = if at >= self.from as usize {
                hi
            } else {
                let i = self.seg_of(at as u32);
                let next = self.segs.get(i + 1).map_or(self.from as usize, |(first, _)| *first as usize);
                next.min(hi)
            };
            f(self.range(at, end, k));
            at = end;
        }
    }

    /// The values of rows `lo..hi`, copied into one vector.
    pub(super) fn to_vec(&self, lo: usize, hi: usize, k: usize) -> Vec<i64> {
        let mut out = Vec::with_capacity((hi - lo) * k);
        self.each_range(lo, hi, k, |s| out.extend_from_slice(s));
        out
    }

    /// The rows as one slice, when they are one: no frozen segment, or no tail and one segment.
    pub(super) fn as_flat(&self) -> Option<&[i64]> {
        match (&self.segs[..], self.tail.is_empty()) {
            ([], _) => Some(&self.tail),
            ([(0, s)], true) => Some(s),
            _ => None,
        }
    }

    /// These `n` rows with the tail frozen into a segment of its own (moved, not copied): as a closed relation holds
    /// them, for a later run to share (`shared`).
    pub(super) fn freeze(mut self, n: usize) -> Rows {
        if !self.tail.is_empty() {
            let tail = std::mem::take(&mut self.tail);
            self.segs.push((self.from, Arc::new(tail)));
        }
        self.from = n as u32;
        while self.segs.len() > MAX_SEGMENTS {
            // The newest two, merged: a session's edits add small segments after one large one.
            let (_, b) = self.segs.pop().expect("two segments");
            let (first, a) = self.segs.pop().expect("two segments");
            let mut both = Vec::with_capacity(a.len() + b.len());
            both.extend_from_slice(&a);
            both.extend_from_slice(&b);
            self.segs.push((first, Arc::new(both)));
        }
        self
    }

    /// The `n` rows of a frozen relation, shared: its segments taken (and its tail copied, if it has one), with an
    /// empty tail from `n` on, for a relation that starts as them.
    pub(super) fn shared(&self, n: usize) -> Rows {
        let mut segs = self.segs.clone();
        if !self.tail.is_empty() {
            segs.push((self.from, Arc::new(self.tail.clone())));
        }
        Rows { segs, from: n as u32, tail: Vec::new() }
    }

    /// Whether these rows share a frozen segment with `other` (a relation started from it).
    pub(super) fn shares(&self, other: &Rows) -> bool {
        self.segs.iter().any(|(_, a)| other.segs.iter().any(|(_, b)| Arc::ptr_eq(a, b)))
    }

    #[inline]
    fn seg_of(&self, id: u32) -> usize {
        self.segs.partition_point(|(first, _)| *first <= id) - 1
    }
}

impl RowData for Rows {
    #[inline]
    fn row(&self, id: u32, k: usize) -> &[i64] {
        if id >= self.from {
            return &self.tail[(id - self.from) as usize * k..][..k];
        }
        let (first, s) = match &self.segs[..] {
            [only] => only,
            segs => &segs[self.seg_of(id)],
        };
        &s[(id - first) as usize * k..][..k]
    }
}

impl RowData for Vec<i64> {
    #[inline]
    fn row(&self, id: u32, k: usize) -> &[i64] {
        &self[id as usize * k..][..k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frozen relation's rows are shared by one started from it, which pushes its own after them; freezing that
    /// one moves its tail into a segment, and both read every row by id.
    #[test]
    fn started_rows_share_the_kept_segments_and_freeze_their_own() {
        let k = 2;
        let kept = Rows::from_vec((0..20).collect()).freeze(10);
        let mut next = kept.shared(10);
        assert!(next.shares(&kept));
        next.extend_from_slice(&[100, 101]);
        next.extend_from_slice(&[102, 103]);
        assert_eq!(next.row(3, k), &[6, 7]);
        assert_eq!(next.row(11, k), &[102, 103]);
        assert_eq!(next.to_vec(9, 12, k), vec![18, 19, 100, 101, 102, 103]);
        let closed = next.freeze(12);
        assert!(closed.shares(&kept), "the kept segment is not copied at close");
        assert_eq!(closed.row(10, k), &[100, 101]);
        assert_eq!(closed.range(10, 12, k), &[100, 101, 102, 103]);
        // Edits add small segments; past the most held, the newest are merged and every row stays where it was.
        let mut rows = closed;
        let mut n = 12;
        for e in 0..20 {
            let mut s = rows.shared(n);
            s.extend_from_slice(&[1000 + e, 2000 + e]);
            n += 1;
            rows = s.freeze(n);
        }
        assert!(rows.segs.len() <= MAX_SEGMENTS);
        assert!(rows.shares(&kept), "the first segment is kept, not merged");
        assert_eq!(rows.row(5, k), &[10, 11]);
        assert_eq!(rows.row(12, k), &[1000, 2000]);
        assert_eq!(rows.row(31, k), &[1019, 2019]);
        assert_eq!(rows.len(), n * k);
    }
}
