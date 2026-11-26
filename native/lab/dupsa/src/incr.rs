//! Incremental duplicated blocks: the findings kept up to date as documents change, without rebuilding the suffix
//! array.
//!
//! Why it can be incremental. A finding depends only on which windows of `w` units are equal (`report.rs`): a
//! position is duplicated when another position's window is the same, and a block's other places are its first
//! window's. Nothing past depth `w` matters, so the suffix array is needed only to depth `w`: the windows, sorted,
//! with a bit for "the same window as the one before" (`eq`). A window is local to its `w` units, so an edit changes
//! only the windows that overlap it; every other window of the edited document keeps its content and only moves.
//!
//! The index is a log-structured merge of two levels:
//! - the **base**: every full window of a snapshot of the documents, in window order, with its class (a run of equal
//!   windows), each position's rank, and each class's first rank;
//! - the **overlay**, per document changed since the base: the base windows the change overlaps are tombstoned
//!   (`dead`), the new windows that overlap it are looked up in the base (a binary search over the windows) and
//!   added to the class they equal, or to a fresh class when no base window is the same; the rest of the document's
//!   windows are the base's, at positions moved by the change's length.
//!
//! An overlay entry is a function of the base and the document's current version only (the change is the two
//! versions' common prefix and suffix trimmed off), so re-editing a document replaces its entry, and a run that
//! starts from the base needs only the documents changed since it.
//!
//! After a batch of changes the blocks are recomputed in the documents whose windows share a class that gained or
//! lost a member (its count may have crossed 2), and in the changed documents; a block holds its first window's
//! class, and its other places are read from the class when reported. **Compaction** merges the overlay into a new
//! base in one pass over the base (tombstones dropped, added members placed by their class or insertion rank, no
//! window compared or sorted again).

use crate::multi::Docs;
use std::collections::HashMap;

const NONE: u32 = u32::MAX;
const FRESH: u32 = 1 << 31;

/// Where each document's version is read: one of several sources (the base snapshot, each batch's new versions),
/// and its document there. A deleted document has no source.
#[derive(Clone)]
pub struct Current<'a> {
    pub sources: Vec<&'a dyn Docs>,
    pub at: Vec<(u32, u32)>,
}

impl<'a> Current<'a> {
    pub fn of(docs: &'a dyn Docs) -> Self {
        Self { sources: vec![docs], at: (0..docs.count() as u32).map(|d| (0, d)).collect() }
    }
}

impl Docs for Current<'_> {
    fn count(&self) -> usize {
        self.at.len()
    }
    fn doc_len(&self, d: usize) -> usize {
        let (s, n) = self.at[d];
        if s == NONE { 0 } else { self.sources[s as usize].doc_len(n as usize) }
    }
    #[inline(always)]
    fn at(&self, d: usize, j: usize) -> u32 {
        let (s, n) = self.at[d];
        self.sources[s as usize].at(n as usize, j)
    }
    fn upper(&self) -> u32 {
        self.sources.iter().map(|s| s.upper()).max().unwrap_or(0)
    }
}

/// Every full window of a snapshot, in window order (a suffix array to depth `w`). Positions are document-major,
/// without separators: document `d`'s unit `j` is `offsets[d] + j`.
pub struct Base {
    pub w: u32,
    pub offsets: Vec<u32>,
    /// The positions of the full windows (those with `w` units left in their document), in window order.
    pub sa: Vec<u32>,
    /// Bit `r`: `sa[r]`'s window is `sa[r - 1]`'s.
    pub eq: Vec<u64>,
    /// Per position: its window's rank, or NONE for a short window.
    pub rank: Vec<u32>,
    /// Per rank: its class (a run of equal windows).
    pub class_of: Vec<u32>,
    /// Per class: its first rank; then `sa.len()`.
    pub class_lo: Vec<u32>,
}

fn bit(v: &[u64], i: usize) -> bool {
    v[i / 64] >> (i % 64) & 1 == 1
}

fn set_bit(v: &mut [u64], i: usize, on: bool) {
    if on {
        v[i / 64] |= 1 << (i % 64);
    } else {
        v[i / 64] &= !(1 << (i % 64));
    }
}

pub fn put32(out: &mut Vec<u8>, v: &[u32]) {
    out.extend_from_slice(&(v.len() as u64).to_le_bytes());
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// An array `put32` wrote (or, with `fixed` > 0, that many values without a length).
pub fn get32(b: &[u8], at: &mut usize, fixed: usize) -> Vec<u32> {
    let n = if fixed > 0 {
        fixed
    } else {
        let n = u64::from_le_bytes(b[*at..*at + 8].try_into().unwrap()) as usize;
        *at += 8;
        n
    };
    let v = b[*at..*at + 4 * n].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    *at += 4 * n;
    v
}

fn offsets_of(docs: &dyn Docs) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(docs.count() + 1);
    let mut at = 0u64;
    offsets.push(0);
    for d in 0..docs.count() {
        at += docs.doc_len(d) as u64;
        offsets.push(u32::try_from(at).expect("2^32 units or more"));
    }
    offsets
}

impl Base {
    /// From the windows in order and their `eq` bits: the ranks, classes and class starts.
    fn from_order(w: u32, offsets: Vec<u32>, sa: Vec<u32>, eq: Vec<u64>) -> Self {
        let n = *offsets.last().unwrap() as usize;
        let mut rank = vec![NONE; n];
        let mut class_of = Vec::with_capacity(sa.len());
        let mut class_lo = Vec::new();
        for (r, &p) in sa.iter().enumerate() {
            rank[p as usize] = r as u32;
            if r == 0 || !bit(&eq, r) {
                class_lo.push(r as u32);
            }
            class_of.push(class_lo.len() as u32 - 1);
        }
        class_lo.push(sa.len() as u32);
        Self { w, offsets, sa, eq, rank, class_of, class_lo }
    }

    /// From a whole suffix array (SA-IS, `MultiSuffixArray`): its full windows, with `eq` from the LCP (the common
    /// prefix of two kept entries is the least LCP between them).
    pub fn from_suffix_array(docs: &dyn Docs, a: &crate::multi::MultiSuffixArray, w: u32) -> Self {
        let offsets = offsets_of(docs);
        let mut sa = Vec::new();
        let mut eq = Vec::new();
        let mut run = u32::MAX;
        for (r, &(d, j)) in a.entries.iter().enumerate() {
            run = run.min(a.lcp[r]);
            if j as usize + w as usize > docs.doc_len(d as usize) {
                continue;
            }
            let k = sa.len();
            if k % 64 == 0 {
                eq.push(0);
            }
            set_bit(&mut eq, k, k > 0 && run >= w);
            sa.push(offsets[d as usize] + j);
            run = u32::MAX;
        }
        Self::from_order(w, offsets, sa, eq)
    }

    /// By prefix doubling to depth `w` (Manber and Myers, with Larsson and Sadakane's re-sort of the unsorted groups
    /// only): each symbol read once; ranks of prefixes of length `len` give those of `len + h` by sorting each group
    /// of equal ranks on the rank `h` further on; stops at `w`, or when every group is one position.
    pub fn doubling(docs: &dyn Docs, w: u32) -> Self {
        let offsets = offsets_of(docs);
        let n = *offsets.last().unwrap() as usize;
        // Per position, its document's end (working state of the sort).
        let mut end = vec![0u32; n];
        let mut r = vec![0u32; n];
        for d in 0..docs.count() {
            let (lo, hi) = (offsets[d] as usize, offsets[d + 1] as usize);
            for p in lo..hi {
                end[p] = hi as u32;
                r[p] = docs.at(d, p - lo) + 1;
            }
        }
        // Counting sort by symbol.
        let upper = docs.upper() as usize + 2;
        let mut count = vec![0u32; upper + 1];
        for &x in &r {
            count[x as usize + 1] += 1;
        }
        for k in 1..count.len() {
            count[k] += count[k - 1];
        }
        let mut sa = vec![0u32; n];
        for p in 0..n {
            let x = r[p] as usize;
            sa[count[x] as usize] = p as u32;
            count[x] += 1;
        }
        drop(count);
        // Ranks are a group's first index + 1 (0 is "past the document's end"); groups of more than one are open.
        let mut groups: Vec<(u32, u32)> = Vec::new();
        let mut k = 0;
        while k < n {
            let x = r[sa[k] as usize];
            let mut e = k + 1;
            while e < n && r[sa[e] as usize] == x {
                e += 1;
            }
            for &p in &sa[k..e] {
                r[p as usize] = k as u32 + 1;
            }
            if e - k > 1 {
                groups.push((k as u32, e as u32));
            }
            k = e;
        }
        let mut len = 1u32;
        let mut keyed: Vec<(u32, u32)> = Vec::new();
        while len < w && !groups.is_empty() {
            let h = len.min(w - len);
            // Every open group's keys from this round's ranks, before any is changed.
            keyed.clear();
            for &(s, e) in &groups {
                for &p in &sa[s as usize..e as usize] {
                    let q = p + h;
                    keyed.push((if q < end[p as usize] { r[q as usize] } else { 0 }, p));
                }
            }
            let mut next = Vec::new();
            let mut at = 0usize;
            for &(s, e) in &groups {
                let g = &mut keyed[at..at + (e - s) as usize];
                at += (e - s) as usize;
                g.sort_unstable();
                let mut i = 0;
                while i < g.len() {
                    let mut j = i + 1;
                    while j < g.len() && g[j].0 == g[i].0 {
                        j += 1;
                    }
                    for (x, &(_, p)) in g[i..j].iter().enumerate() {
                        sa[s as usize + i + x] = p;
                        r[p as usize] = s + i as u32 + 1;
                    }
                    if j - i > 1 {
                        next.push((s + i as u32, s + j as u32));
                    }
                    i = j;
                }
            }
            groups = next;
            len += h;
        }
        let deep = len >= w;
        let mut full = Vec::new();
        let mut eq = Vec::new();
        let mut last = NONE;
        for &p in &sa {
            if p + w > end[p as usize] {
                continue;
            }
            let k = full.len();
            if k % 64 == 0 {
                eq.push(0);
            }
            set_bit(&mut eq, k, k > 0 && deep && r[p as usize] == last);
            last = r[p as usize];
            full.push(p);
        }
        Self::from_order(w, offsets, full, eq)
    }

    /// The base as bytes (each array little-endian, its length first), for a later run to read back.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.w.to_le_bytes());
        for v in [&self.offsets, &self.sa, &self.rank, &self.class_of, &self.class_lo] {
            put32(out, v);
        }
        out.extend_from_slice(&(self.eq.len() as u64).to_le_bytes());
        for x in &self.eq {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    pub fn read(b: &[u8], at: &mut usize) -> Self {
        let w = get32(b, at, 1)[0];
        let (offsets, sa, rank, class_of, class_lo) = (get32(b, at, 0), get32(b, at, 0), get32(b, at, 0), get32(b, at, 0), get32(b, at, 0));
        let n = u64::from_le_bytes(b[*at..*at + 8].try_into().unwrap()) as usize;
        *at += 8;
        let eq = b[*at..*at + 8 * n].chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        *at += 8 * n;
        Self { w, offsets, sa, eq, rank, class_of, class_lo }
    }

    pub fn classes(&self) -> usize {
        self.class_lo.len() - 1
    }

    fn doc_of(&self, p: u32) -> usize {
        self.offsets.partition_point(|&o| o <= p) - 1
    }

    /// Bytes the base holds (its arrays).
    pub fn bytes(&self) -> usize {
        4 * (self.offsets.len() + self.sa.len() + self.rank.len() + self.class_of.len() + self.class_lo.len()) + 8 * self.eq.len()
    }
}

/// A document changed since the base: its change (the units `a..b_old` of the base version became `a..b_new`),
/// and what the overlay holds for it.
struct Edit {
    a: u32,
    b_old: u32,
    b_new: u32,
    /// Its tombstoned base ranks.
    dead: Vec<u32>,
    /// The class of each new window, from position `lo_new`.
    lo_new: u32,
    cls: Vec<u32>,
}

struct Fresh {
    /// The base rank its windows sort before.
    at: u32,
    members: Vec<(u32, u32)>,
}

/// A block: units `first..end` of its document, and its first window's class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Blk {
    pub first: u32,
    pub end: u32,
    pub class: u32,
}

/// What one batch did.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub docs: usize,
    pub dead: usize,
    pub added: usize,
    pub fresh: usize,
    pub touched: usize,
    pub affected: usize,
    pub recomputed_units: usize,
}

/// The change between two versions of a document, as kept for mapping old places to new: units `a..b_old` became
/// `a..b_new`.
#[derive(Clone, Copy, Debug)]
pub struct Change {
    pub a: u32,
    pub b_old: u32,
    pub b_new: u32,
}

impl Change {
    /// Where an old window of `w` units at `u` is now, if the change did not touch it.
    pub fn map(&self, u: u32, w: u32) -> Option<u32> {
        if u + w <= self.a {
            Some(u)
        } else if u >= self.b_old {
            Some(u - self.b_old + self.b_new)
        } else {
            None
        }
    }
}

fn trim(old: &dyn Docs, od: usize, new: &dyn Docs, nd: usize, old_len: usize, new_len: usize) -> Change {
    let m = old_len.min(new_len);
    let mut a = 0;
    while a < m && old.at(od, a) == new.at(nd, a) {
        a += 1;
    }
    let mut s = 0;
    while s < m - a && old.at(od, old_len - 1 - s) == new.at(nd, new_len - 1 - s) {
        s += 1;
    }
    Change { a: a as u32, b_old: (old_len - s) as u32, b_new: (new_len - s) as u32 }
}

pub struct Index<'a> {
    pub w: u32,
    pub base: Base,
    base_docs: Current<'a>,
    pub cur: Current<'a>,
    /// Per base rank: tombstoned.
    dead: Vec<u64>,
    /// Per base class: its live base members.
    count: Vec<u32>,
    extra: HashMap<u32, Vec<(u32, u32)>>,
    fresh: Vec<Fresh>,
    fresh_by_hash: HashMap<u64, Vec<u32>>,
    edits: HashMap<u32, Edit>,
    /// Per document, its blocks.
    pub blocks: Vec<Vec<Blk>>,
    touched: Vec<u32>,
    touched_set: std::collections::HashSet<u32>,
    changed: Vec<u32>,
    /// Each document's change since the last batch (for mapping the last findings onto the new versions).
    pub last_changes: HashMap<u32, Change>,
    /// Re-add a changed document's every window (the whole document as the change), not only those it overlaps.
    pub whole_doc: bool,
    pub stats: Stats,
}

impl<'a> Index<'a> {
    /// The blocks as bytes: per document their count, then each block's three values.
    pub fn write_blocks(&self, out: &mut Vec<u8>) {
        let mut v = vec![self.blocks.len() as u32];
        for bs in &self.blocks {
            v.push(bs.len() as u32);
            for b in bs {
                v.extend_from_slice(&[b.first, b.end, b.class]);
            }
        }
        put32(out, &v);
    }

    /// An index over a base read back, with the blocks a run before computed on it (none are recomputed).
    pub fn with_blocks(base: Base, docs: Current<'a>, b: &[u8], at: &mut usize) -> Self {
        let v = get32(b, at, 0);
        let mut blocks = Vec::with_capacity(v[0] as usize);
        let mut k = 1;
        for _ in 0..v[0] {
            let n = v[k] as usize;
            k += 1;
            blocks.push((0..n).map(|i| Blk { first: v[k + 3 * i], end: v[k + 3 * i + 1], class: v[k + 3 * i + 2] }).collect());
            k += 3 * n;
        }
        let mut ix = Self::empty(base, docs);
        ix.blocks = blocks;
        ix
    }

    pub fn new(base: Base, docs: Current<'a>) -> Self {
        let mut ix = Self::empty(base, docs);
        for d in 0..ix.cur.count() {
            ix.blocks[d] = ix.doc_blocks(d);
        }
        ix
    }

    fn empty(base: Base, docs: Current<'a>) -> Self {
        let count = (0..base.classes()).map(|c| base.class_lo[c + 1] - base.class_lo[c]).collect();
        Self {
            w: base.w,
            dead: vec![0; base.sa.len().div_ceil(64)],
            count,
            base,
            base_docs: docs.clone(),
            blocks: vec![Vec::new(); docs.count()],
            cur: docs,
            extra: HashMap::new(),
            fresh: Vec::new(),
            fresh_by_hash: HashMap::new(),
            edits: HashMap::new(),
            touched: Vec::new(),
            touched_set: Default::default(),
            changed: Vec::new(),
            last_changes: HashMap::new(),
            whole_doc: false,
            stats: Stats::default(),
        }
    }

    fn touch(&mut self, c: u32) {
        if self.touched_set.insert(c) {
            self.touched.push(c);
        }
    }

    fn live_count(&self, c: u32) -> usize {
        if c & FRESH != 0 {
            self.fresh[(c & !FRESH) as usize].members.len()
        } else {
            self.count[c as usize] as usize + self.extra.get(&c).map_or(0, Vec::len)
        }
    }

    /// The class of the window at unit `p` of document `d` as it is now, or None for a short window.
    fn class_at(&self, d: usize, p: u32) -> Option<u32> {
        let w = self.w;
        if p as usize + w as usize > self.cur.doc_len(d) {
            return None;
        }
        let q = match self.edits.get(&(d as u32)) {
            Some(e) if p + w > e.a && p < e.b_new => return Some(e.cls[(p - e.lo_new) as usize]),
            Some(e) if p >= e.b_new => p - e.b_new + e.b_old,
            _ => p,
        };
        let r = self.base.rank[(self.base.offsets[d] + q) as usize];
        Some(self.base.class_of[r as usize])
    }

    fn doc_blocks(&self, d: usize) -> Vec<Blk> {
        let len = self.cur.doc_len(d) as u32;
        let mut out = Vec::new();
        if len < self.w {
            return out;
        }
        let dup = |p: u32| self.class_at(d, p).filter(|&c| self.live_count(c) >= 2);
        let mut p = 0;
        while p + self.w <= len {
            let Some(c) = dup(p) else {
                p += 1;
                continue;
            };
            let first = p;
            while p + 1 + self.w <= len && dup(p + 1).is_some() {
                p += 1;
            }
            out.push(Blk { first, end: p + self.w, class: c });
            p += 1;
        }
        out
    }

    /// Where a base position is now: (document, unit), or None when tombstoned.
    fn base_place(&self, r: u32) -> Option<(u32, u32)> {
        if bit(&self.dead, r as usize) {
            return None;
        }
        let p = self.base.sa[r as usize];
        let d = self.base.doc_of(p);
        let u = p - self.base.offsets[d];
        Some(match self.edits.get(&(d as u32)) {
            Some(e) if u >= e.b_old => (d as u32, u - e.b_old + e.b_new),
            _ => (d as u32, u),
        })
    }

    /// Every live member of a class, (document, unit) as they are now.
    pub fn members(&self, c: u32) -> Vec<(u32, u32)> {
        if c & FRESH != 0 {
            return self.fresh[(c & !FRESH) as usize].members.clone();
        }
        let (lo, hi) = (self.base.class_lo[c as usize], self.base.class_lo[c as usize + 1]);
        let mut out: Vec<(u32, u32)> = (lo..hi).filter_map(|r| self.base_place(r)).collect();
        if let Some(x) = self.extra.get(&c) {
            out.extend_from_slice(x);
        }
        out
    }

    /// The base window at rank `r` against the current window at unit `p` of `d`, from unit `from` on: the order,
    /// and how many units they share.
    fn cmp_base(&self, r: u32, d: usize, p: u32, from: u32) -> (std::cmp::Ordering, u32) {
        let bp = self.base.sa[r as usize];
        let bd = self.base.doc_of(bp);
        let bj = (bp - self.base.offsets[bd]) as usize;
        for k in from..self.w {
            let (x, y) = (self.base_docs.at(bd, bj + k as usize), self.cur.at(d, (p + k) as usize));
            if x != y {
                return (x.cmp(&y), k);
            }
        }
        (std::cmp::Ordering::Equal, self.w)
    }

    /// The base class of the window at unit `p` of `d`, or the rank it would sort before: a binary search over the
    /// windows, skipping the units both bounds share with it.
    fn lookup(&self, d: usize, p: u32) -> Result<u32, u32> {
        let (mut lo, mut hi) = (0u32, self.base.sa.len() as u32);
        let (mut llo, mut lhi) = (0u32, 0u32);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (ord, m) = self.cmp_base(mid, d, p, llo.min(lhi));
            if ord == std::cmp::Ordering::Less {
                lo = mid + 1;
                llo = m;
            } else {
                hi = mid;
                lhi = m;
            }
        }
        if (lo as usize) < self.base.sa.len() && lhi == self.w {
            Ok(self.base.class_of[lo as usize])
        } else {
            Err(lo)
        }
    }

    fn window_hash(&self, d: usize, p: u32) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for k in 0..self.w as usize {
            h = (h ^ u64::from(self.cur.at(d, p as usize + k))).wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    fn same_window(&self, (d1, p1): (u32, u32), d: usize, p: u32) -> bool {
        (0..self.w as usize).all(|k| self.cur.at(d1 as usize, p1 as usize + k) == self.cur.at(d, p as usize + k))
    }

    /// Add the window at unit `p` of `d` (as it is now) to its class.
    fn add(&mut self, d: usize, p: u32) -> u32 {
        let c = match self.lookup(d, p) {
            Ok(c) => {
                self.extra.entry(c).or_default().push((d as u32, p));
                c
            }
            Err(at) => {
                let h = self.window_hash(d, p);
                let hit = self.fresh_by_hash.get(&h).and_then(|ids| {
                    ids.iter().copied().find(|&f| self.same_window(self.fresh[f as usize].members[0], d, p))
                });
                let f = match hit {
                    Some(f) => f,
                    None => {
                        self.fresh.push(Fresh { at, members: Vec::new() });
                        let f = self.fresh.len() as u32 - 1;
                        self.fresh_by_hash.entry(h).or_default().push(f);
                        self.stats.fresh += 1;
                        f
                    }
                };
                self.fresh[f as usize].members.push((d as u32, p));
                f | FRESH
            }
        };
        self.touch(c);
        self.stats.added += 1;
        c
    }

    fn remove(&mut self, c: u32, place: (u32, u32)) {
        if c & FRESH != 0 {
            let f = (c & !FRESH) as usize;
            let m = &mut self.fresh[f].members;
            m.swap_remove(m.iter().position(|&x| x == place).expect("a member"));
            if m.is_empty() {
                // An empty fresh class has no window to compare: it is taken out of the hash.
                for ids in self.fresh_by_hash.values_mut() {
                    ids.retain(|&x| x as usize != f);
                }
            }
        } else {
            let m = self.extra.get_mut(&c).expect("added members");
            m.swap_remove(m.iter().position(|&x| x == place).expect("a member"));
        }
        self.touch(c);
    }

    /// Take a document's overlay entry out: its tombstones lifted, its added windows removed.
    fn revert(&mut self, d: u32) {
        let Some(e) = self.edits.remove(&d) else { return };
        for &r in &e.dead {
            set_bit(&mut self.dead, r as usize, false);
            let c = self.base.class_of[r as usize];
            self.count[c as usize] += 1;
            self.touch(c);
        }
        for (k, &c) in e.cls.iter().enumerate() {
            self.remove(c, (d, e.lo_new + k as u32));
        }
    }

    /// Document `d`'s version is now document `nd` of `source` (`NONE` for deleted); `d` past the end adds one.
    pub fn set(&mut self, d: u32, version: (u32, u32)) {
        let du = d as usize;
        while self.cur.count() <= du {
            self.cur.at.push((NONE, 0));
            self.blocks.push(Vec::new());
        }
        // The change since the last version, for mapping the last findings.
        let (prev_len, new_len) = (self.cur.doc_len(du), {
            let (s, n) = version;
            if s == NONE { 0 } else { self.cur.sources[s as usize].doc_len(n as usize) }
        });
        let last = if self.cur.at[du].0 == NONE || version.0 == NONE {
            Change { a: 0, b_old: prev_len as u32, b_new: new_len as u32 }
        } else {
            let (s, n) = version;
            trim(&self.cur, du, self.cur.sources[s as usize], n as usize, prev_len, new_len)
        };
        self.last_changes.insert(d, last);
        self.revert(d);
        self.cur.at[du] = version;
        self.changed.push(d);
        self.stats.docs += 1;
        let old_len = if du < self.base_docs.count() { self.base_docs.doc_len(du) } else { 0 };
        let mut ch = if old_len == 0 || new_len == 0 {
            Change { a: 0, b_old: old_len as u32, b_new: new_len as u32 }
        } else {
            trim(&self.base_docs, du, &self.cur, du, old_len, new_len)
        };
        if self.whole_doc {
            ch = Change { a: 0, b_old: old_len as u32, b_new: new_len as u32 };
        }
        if ch.a == ch.b_old && ch.a == ch.b_new {
            return; // the base version again
        }
        let w = self.w;
        let lo = ch.a.saturating_sub(w - 1);
        let mut dead = Vec::new();
        if old_len as u32 >= w {
            for p in lo..ch.b_old.min(old_len as u32 - w + 1) {
                let r = self.base.rank[(self.base.offsets[du] + p) as usize];
                set_bit(&mut self.dead, r as usize, true);
                let c = self.base.class_of[r as usize];
                self.count[c as usize] -= 1;
                self.touch(c);
                dead.push(r);
            }
        }
        self.stats.dead += dead.len();
        // The overlay entry goes in before its windows are added, so that a window's lookup reads the new version.
        self.edits.insert(d, Edit { a: ch.a, b_old: ch.b_old, b_new: ch.b_new, dead, lo_new: lo, cls: Vec::new() });
        let mut cls = Vec::new();
        if new_len as u32 >= w {
            for p in lo..ch.b_new.min(new_len as u32 - w + 1) {
                cls.push(self.add(du, p));
            }
        }
        self.edits.get_mut(&d).unwrap().cls = cls;
    }

    /// Recompute the blocks the batch's changes can have changed: in the changed documents, and in every document
    /// with a member of a class that gained or lost one.
    pub fn finish(&mut self) -> Stats {
        let mut affected: Vec<u32> = std::mem::take(&mut self.changed);
        for &c in &self.touched {
            affected.extend(self.members(c).into_iter().map(|(d, _)| d));
        }
        // A document whose blocks hold a touched class lost the members it had there (they may all be gone).
        for (d, bs) in self.blocks.iter().enumerate() {
            if bs.iter().any(|b| self.touched_set.contains(&b.class)) {
                affected.push(d as u32);
            }
        }
        affected.sort_unstable();
        affected.dedup();
        self.stats.touched = self.touched.len();
        self.stats.affected = affected.len();
        for &d in &affected {
            self.stats.recomputed_units += self.cur.doc_len(d as usize);
            self.blocks[d as usize] = self.doc_blocks(d as usize);
        }
        self.touched.clear();
        self.touched_set.clear();
        std::mem::take(&mut self.stats)
    }

    /// Every block with its other places, as `report::blocks` gives them for the current versions.
    pub fn findings(&self) -> Vec<crate::report::Block> {
        let mut out = Vec::new();
        for (d, bs) in self.blocks.iter().enumerate() {
            for b in bs {
                let mut others: Vec<(u32, u32)> = self.members(b.class).into_iter().filter(|&x| x != (d as u32, b.first)).collect();
                others.sort_unstable();
                out.push(crate::report::Block { doc: d as u32, first: b.first, end: b.end, others });
            }
        }
        out
    }

    /// Bytes the overlay holds.
    pub fn overlay_size(&self) -> (usize, usize) {
        let dead: usize = self.edits.values().map(|e| e.dead.len()).sum();
        let added: usize = self.edits.values().map(|e| e.cls.len()).sum();
        (dead, added)
    }

    /// Merge the overlay into a new base over the current versions: one pass over the base in window order, the
    /// tombstoned windows dropped, each added window placed in its class, each fresh class at its insertion rank.
    pub fn compact(&mut self) {
        let w = self.w;
        let offsets = offsets_of(&self.cur);
        let mut sa = Vec::with_capacity(self.base.sa.len() + 1024);
        let mut eq: Vec<u64> = Vec::with_capacity(self.base.eq.len() + 16);
        // Old class (base or fresh) to new.
        let mut map_base = vec![NONE; self.base.classes()];
        let mut map_fresh = vec![NONE; self.fresh.len()];
        let mut new_classes = 0u32;
        let mut emit = |sa: &mut Vec<u32>, eq: &mut Vec<u64>, places: &mut dyn Iterator<Item = (u32, u32)>| -> bool {
            let mut any = false;
            for (d, u) in places {
                let k = sa.len();
                if k % 64 == 0 {
                    eq.push(0);
                }
                set_bit(eq, k, any);
                sa.push(offsets[d as usize] + u);
                any = true;
            }
            any
        };
        // Fresh classes by insertion rank, ordered among themselves by their windows.
        let mut fresh_at: Vec<u32> = (0..self.fresh.len() as u32).filter(|&f| !self.fresh[f as usize].members.is_empty()).collect();
        let cmp_fresh = |x: &u32, y: &u32| {
            let (fx, fy) = (&self.fresh[*x as usize], &self.fresh[*y as usize]);
            fx.at.cmp(&fy.at).then_with(|| {
                let ((d1, p1), (d2, p2)) = (fx.members[0], fy.members[0]);
                (0..w as usize)
                    .map(|k| self.cur.at(d1 as usize, p1 as usize + k).cmp(&self.cur.at(d2 as usize, p2 as usize + k)))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        };
        fresh_at.sort_unstable_by(cmp_fresh);
        let mut fi = 0;
        for c in 0..=self.base.classes() {
            let lo = self.base.class_lo[c];
            while fi < fresh_at.len() && self.fresh[fresh_at[fi] as usize].at == lo {
                let f = fresh_at[fi];
                let mut members = self.fresh[f as usize].members.clone();
                members.sort_unstable();
                if emit(&mut sa, &mut eq, &mut members.into_iter()) {
                    map_fresh[f as usize] = new_classes;
                    new_classes += 1;
                }
                fi += 1;
            }
            if c == self.base.classes() {
                break;
            }
            let hi = self.base.class_lo[c + 1];
            let mut places: Vec<(u32, u32)> = (lo..hi).filter_map(|r| self.base_place(r)).collect();
            if let Some(x) = self.extra.get(&(c as u32)) {
                places.extend_from_slice(x);
            }
            if emit(&mut sa, &mut eq, &mut places.into_iter()) {
                map_base[c] = new_classes;
                new_classes += 1;
            }
        }
        let base = Base::from_order(w, offsets, sa, eq);
        debug_assert_eq!(base.classes() as u32, new_classes);
        for bs in &mut self.blocks {
            for b in bs.iter_mut() {
                b.class = if b.class & FRESH != 0 { map_fresh[(b.class & !FRESH) as usize] } else { map_base[b.class as usize] };
                debug_assert_ne!(b.class, NONE);
            }
        }
        self.count = (0..base.classes()).map(|c| base.class_lo[c + 1] - base.class_lo[c]).collect();
        self.dead = vec![0; base.sa.len().div_ceil(64)];
        self.base = base;
        self.base_docs = self.cur.clone();
        self.extra.clear();
        self.fresh.clear();
        self.fresh_by_hash.clear();
        self.edits.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi::MultiSuffixArray;

    struct Strs(Vec<Vec<u32>>, u32);

    impl Docs for Strs {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn doc_len(&self, d: usize) -> usize {
            self.0[d].len()
        }
        fn at(&self, d: usize, j: usize) -> u32 {
            self.0[d][j]
        }
        fn upper(&self) -> u32 {
            self.1
        }
    }

    /// A small deterministic generator (xorshift).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn rebuild(docs: &dyn Docs, w: u32) -> Vec<crate::report::Block> {
        let a = MultiSuffixArray::new(docs);
        crate::report::blocks(&a, w)
    }

    fn key(bs: Vec<crate::report::Block>) -> Vec<(u32, u32, u32, Vec<(u32, u32)>)> {
        let mut v: Vec<_> = bs.into_iter().map(|b| (b.doc, b.first, b.end, b.others)).collect();
        v.sort();
        v
    }

    /// Documents made of a few shared runs, so that windows repeat.
    fn doc(rng: &mut Rng, runs: &[Vec<u32>], alpha: u32) -> Vec<u32> {
        let mut d = Vec::new();
        for _ in 0..rng.below(5) {
            if rng.below(2) == 0 {
                d.extend_from_slice(&runs[rng.below(runs.len())]);
            } else {
                for _ in 0..rng.below(6) {
                    d.push(rng.below(alpha as usize) as u32);
                }
            }
        }
        d
    }

    #[test]
    fn doubling_has_the_suffix_arrays_classes() {
        let mut rng = Rng(7);
        for round in 0..200 {
            let alpha = 2 + (round % 5) as u32;
            let runs: Vec<Vec<u32>> = (0..3).map(|_| (0..2 + rng.below(8)).map(|_| rng.below(alpha as usize) as u32).collect()).collect();
            let docs = Strs((0..1 + rng.below(6)).map(|_| doc(&mut rng, &runs, alpha)).collect(), alpha);
            let w = 1 + rng.below(6) as u32;
            let a = Base::from_suffix_array(&docs, &MultiSuffixArray::new(&docs), w);
            let b = Base::doubling(&docs, w);
            // Same windows, same classes (the order within a class may differ).
            let classes = |x: &Base| {
                let mut v: Vec<Vec<u32>> = (0..x.classes()).map(|c| {
                    let mut m: Vec<u32> = x.sa[x.class_lo[c] as usize..x.class_lo[c + 1] as usize].to_vec();
                    m.sort_unstable();
                    m
                }).collect();
                v.sort();
                v
            };
            assert_eq!(classes(&a), classes(&b), "round {round}");
            // And the classes in the same order.
            let firsts = |x: &Base| (0..x.classes()).map(|c| *x.sa[x.class_lo[c] as usize..x.class_lo[c + 1] as usize].iter().min().unwrap()).collect::<Vec<_>>();
            let (fa, fb) = (firsts(&a), firsts(&b));
            let pos = |f: &Vec<u32>, x: &Base| f.iter().map(|&p| x.class_of[x.rank[p as usize] as usize]).collect::<Vec<_>>();
            assert_eq!(pos(&fa, &a), pos(&fa, &b));
            let _ = fb;
        }
    }

    thread_local! {
        static SEEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static COMPACTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn edits_give_the_rebuilds_findings() {
        let mut rng = Rng(99);
        for round in 0..300 {
            let alpha = 2 + (round % 4) as u32;
            let runs: Vec<Vec<u32>> = (0..3).map(|_| (0..3 + rng.below(8)).map(|_| rng.below(alpha as usize) as u32).collect()).collect();
            let base = Strs((0..1 + rng.below(5)).map(|_| doc(&mut rng, &runs, alpha)).collect(), alpha);
            let w = 1 + rng.below(5) as u32;
            // Each batch's new versions, kept for the index to read.
            let mut batches: Vec<Box<Strs>> = Vec::new();
            let mut plan: Vec<Vec<(u32, Option<usize>)>> = Vec::new();
            let mut ndocs = base.0.len();
            for _ in 0..4 {
                let mut vs = Vec::new();
                let mut ch = Vec::new();
                for _ in 0..1 + rng.below(3) {
                    let d = rng.below(ndocs + 1) as u32;
                    if d as usize == ndocs {
                        ndocs += 1;
                    }
                    if rng.below(6) == 0 {
                        ch.push((d, None));
                    } else {
                        vs.push(doc(&mut rng, &runs, alpha));
                        ch.push((d, Some(vs.len() - 1)));
                    }
                }
                batches.push(Box::new(Strs(vs, alpha)));
                plan.push(ch);
            }
            let batches: Vec<&'static Strs> = batches.into_iter().map(|b| &*Box::leak(b)).collect();
            let base: &'static Strs = Box::leak(Box::new(base));
            for whole in [false, true] {
                for build in 0..2 {
                    let cur = Current::of(base);
                    let b = if build == 0 { Base::from_suffix_array(&cur, &MultiSuffixArray::new(&cur), w) } else { Base::doubling(&cur, w) };
                    let mut ix = Index::new(b, cur);
                    ix.whole_doc = whole;
                    assert_eq!(key(ix.findings()), key(rebuild(&ix.cur, w)));
                    SEEN.with(|x| x.set(x.get() + ix.findings().len()));
                    for (k, ch) in plan.iter().enumerate() {
                        ix.cur.sources.push(batches[k]);
                        let s = ix.cur.sources.len() as u32 - 1;
                        for &(d, v) in ch {
                            ix.set(d, v.map_or((NONE, 0), |v| (s, v as u32)));
                        }
                        ix.finish();
                        assert_eq!(key(ix.findings()), key(rebuild(&ix.cur, w)), "round {round} batch {k} w {w}");
                        SEEN.with(|x| x.set(x.get() + ix.findings().len()));
                        if rng.below(3) == 0 {
                            ix.compact();
                            COMPACTED.with(|x| x.set(x.get() + 1));
                            assert_eq!(key(ix.findings()), key(rebuild(&ix.cur, w)), "round {round} batch {k} compacted");
                        }
                    }
                }
            }
        }
        let (seen, compacted) = (SEEN.with(|x| x.get()), COMPACTED.with(|x| x.get()));
        eprintln!("{seen} findings compared, {compacted} compactions");
        assert!(seen > 10_000 && compacted > 500);
    }
}
