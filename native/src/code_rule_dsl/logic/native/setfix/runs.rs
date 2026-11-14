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

/// A relation's rows, read by id: one flat vector (`[i64]`, `k` values a row), or the segments a maintained relation
/// shares with the kept one it started as (`super::Rows`, smoking gun 8). Every row lies whole in one segment.
pub(super) trait RowData {
    fn row(&self, id: u32, k: usize) -> &[i64];
}

impl RowData for [i64] {
    #[inline]
    fn row(&self, id: u32, k: usize) -> &[i64] {
        &self[id as usize * k..][..k]
    }
}

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
    /// In an order probed by whole keys (`Index::printed`): each id's key print (`print`), beside it. A whole key's
    /// ids are found among the prints of its window, a word of prints at a time, and only the first and last match
    /// read (`print_span`). Empty otherwise.
    prints: Vec<u8>,
}

/// A key's print: 8 bits of a hash of its values in `cols` (another multiplier than `tag`'s, so a tagged run's span
/// of one tag still spreads over the prints).
#[inline]
fn print_vals(vals: impl Iterator<Item = i64>) -> u8 {
    let h = vals.fold(0x2545_F491_4F6C_DD1Du64, |h, v| (h ^ v as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    (h >> 56) as u8
}

/// A row's print in an order on `cols`.
#[inline]
fn print_of(row: &[i64], cols: &[usize]) -> u8 {
    print_vals(cols.iter().map(|&c| row[c]))
}

/// A whole key's print (its values in the order's column order), as its rows' are.
#[inline]
fn print_of_key(key: &[i64]) -> u8 {
    print_vals(key.iter().copied())
}

/// Each byte of `w` equal to `b`, as its top bit (the lowest set bit marks the first exactly).
#[inline]
fn bytes_eq(w: u64, b: u8) -> u64 {
    let x = w ^ (b as u64 * 0x0101_0101_0101_0101);
    x.wrapping_sub(0x0101_0101_0101_0101) & !x & 0x8080_8080_8080_8080
}

/// The word of `prints` at `at` (bytes past `to` read as `pad`): eight prints a word.
#[inline]
fn word(prints: &[u8], at: usize, to: usize, pad: u8) -> u64 {
    if at + 8 <= to {
        return u64::from_le_bytes(prints[at..at + 8].try_into().expect("eight prints"));
    }
    let mut b = [pad; 8];
    b[..to - at].copy_from_slice(&prints[at..to]);
    u64::from_le_bytes(b)
}

/// The first position in `from..to` whose print is `p` (`eq`) or is not (`!eq`); `to` if none.
#[inline]
fn first_print(prints: &[u8], from: usize, to: usize, p: u8, eq: bool) -> usize {
    let mut at = from;
    while at < to {
        let w = word(prints, at, to, if eq { !p } else { p });
        let m = if eq { bytes_eq(w, p) } else { !bytes_eq(w, p) & 0x8080_8080_8080_8080 };
        if m != 0 {
            return (at + (m.trailing_zeros() / 8) as usize).min(to);
        }
        at += 8;
    }
    to
}

/// The span of positions `lo..hi` whose ids hold the whole key printed `p` (`is_key` reads a position's row): the
/// first print `p` that is the key starts it, and it ends at the last such before the prints stop being `p`. A
/// key's ids are next to each other in the order, so only prints equal by chance (one in 256) are read besides
/// the span's ends.
#[inline]
fn print_span(prints: &[u8], lo: usize, hi: usize, p: u8, is_key: impl Fn(usize) -> bool) -> (usize, usize) {
    let mut at = lo;
    let s = loop {
        let q = first_print(prints, at, hi, p, true);
        if q >= hi {
            return (hi, hi);
        }
        if is_key(q) {
            break q;
        }
        at = q + 1;
    };
    let mut e = first_print(prints, s + 1, hi, p, false);
    while e > s + 1 && !is_key(e - 1) {
        e -= 1;
    }
    (s, e)
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
    /// Probed by whole keys: its runs keep their ids' key prints (`Run::prints`).
    printed: bool,
}

impl Clone for Index {
    fn clone(&self) -> Index {
        Index { cols: self.cols.clone(), runs: self.runs.clone(), rent: AtomicU64::new(0), printed: self.printed }
    }
}

/// Rows `a` and `b` in the order on `cols` (`cmp_vals`), then by id (so a sort is the same whatever order it came in).
#[inline]
fn cmp_rows<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], a: u32, b: u32) -> Ordering {
    let (ra, rb) = (data.row(a, k), data.row(b, k));
    cmp_vals(cols, tagged(cols, k), ra, rb).then_with(|| a.cmp(&b))
}

impl Run {
    fn fenced<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], ids: Vec<u32>) -> Run {
        if tagged(cols, k) {
            let tags = ids.iter().map(|&id| tag(data.row(id, k)[cols[0]])).collect();
            return Run { ids, fences: Vec::new(), tags, prints: Vec::new() };
        }
        let fences = match cols.first() {
            Some(&lead) => ids.iter().step_by(FENCE).map(|&id| data.row(id, k)[lead]).collect(),
            None => Vec::new(),
        };
        Run { ids, fences, tags: Vec::new(), prints: Vec::new() }
    }

    /// This run with its ids' key prints (an order probed by whole keys): one row read an id.
    fn printed<D: RowData + ?Sized>(mut self, data: &D, k: usize, cols: &[usize]) -> Run {
        self.prints = self.ids.iter().map(|&id| print_of(data.row(id, k), cols)).collect();
        self
    }

    /// The ids of `ids` sorted by `cols` (then id), with their tag and first two values held beside them while
    /// they sort (`sort_keyed`).
    fn of<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], mut ids: Vec<u32>) -> Run {
        let tagged = tagged(cols, k);
        let tags = sort_keyed(&mut ids, tagged, cols.len(), |id, i| data.row(id, k)[cols[i]], |a, b| cmp_rows(data, k, cols, a, b));
        if tagged {
            return Run { ids, fences: Vec::new(), tags, prints: Vec::new() };
        }
        Run::fenced(data, k, cols, ids)
    }

    /// The ids whose first `key.len()` columns of `cols` hold `key`.
    #[inline]
    fn find<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], key: &[i64]) -> &[u32] {
        // No columns: every row (a relation of no columns holds one row at most).
        let Some(&lead) = key.first() else { return &self.ids };
        let cmp = |id: u32| -> Ordering {
            let r = data.row(id, k);
            cols.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let (lo, hi) = if !self.tags.is_empty() {
            // The leading value's tag among the tags: only its ids are read.
            tag_span(&self.tags, tag(lead), 0, true)
        } else {
            // The rows of `lead` start in the block before the first fence at or past it, and end before the first
            // fence past it.
            let a = self.fences.partition_point(|&f| f < lead);
            let b = a + self.fences[a..].partition_point(|&f| f == lead);
            (a.saturating_sub(1) * FENCE, (b * FENCE).min(self.ids.len()))
        };
        if !self.prints.is_empty() && key.len() == cols.len() {
            let (s, e) = print_span(&self.prints, lo, hi, print_of_key(key), |i| cmp(self.ids[i]).is_eq());
            return &self.ids[s..e];
        }
        let window = &self.ids[lo..hi];
        let s = window.partition_point(|&id| cmp(id) == Ordering::Less);
        let e = s + window[s..].partition_point(|&id| cmp(id) == Ordering::Equal);
        &window[s..e]
    }

    /// `find`, from where the last key of a sequence was found (`hint`, a position in this run, set past this key's
    /// ids): the tags or fences are searched galloping from there, either way, so a key near the last one (a sorted
    /// batch of keys, decision 0112) reads only what is near, and a far one costs about a search from scratch.
    #[inline]
    fn find_at<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], key: &[i64], hint: &mut u32) -> &[u32] {
        let Some(&lead) = key.first() else { return &self.ids };
        let n = self.ids.len();
        let cmp = |id: u32| -> Ordering {
            let r = data.row(id, k);
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
        if !self.prints.is_empty() && key.len() == cols.len() {
            let (s, e) = print_span(&self.prints, lo, hi, print_of_key(key), |i| cmp(self.ids[i]).is_eq());
            // Past the key's ids, or where it would be: the next key of a sorted batch is after it.
            *hint = if s < e { e } else { lo } as u32;
            return &self.ids[s..e];
        }
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
    fn held_sorted<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], rows: &[i64], order: &[u32], live: &impl Fn(u32) -> bool, held: &mut [bool]) {
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
            let cmp = |id: u32| -> Ordering { cmp_vals(cols, tagged, data.row(id, k), row) };
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
    fn merge<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], a: &Run, b: &Run) -> Run {
        let mut ids = Vec::with_capacity(a.ids.len() + b.ids.len());
        let tagged = !a.tags.is_empty() && !b.tags.is_empty();
        let mut tags = Vec::with_capacity(if tagged { ids.capacity() } else { 0 });
        let printed = !a.prints.is_empty() && !b.prints.is_empty();
        let mut prints = Vec::with_capacity(if printed { ids.capacity() } else { 0 });
        let (mut i, mut j) = (0, 0);
        while i < a.ids.len() && j < b.ids.len() {
            // Tagged, the tags decide unless equal: rows are read only for ids of one tag.
            let first = if tagged && a.tags[i] != b.tags[j] { a.tags[i] < b.tags[j] } else { cmp_rows(data, k, cols, a.ids[i], b.ids[j]).is_le() };
            if first {
                ids.push(a.ids[i]);
                if tagged { tags.push(a.tags[i]); }
                if printed { prints.push(a.prints[i]); }
                i += 1;
            } else {
                ids.push(b.ids[j]);
                if tagged { tags.push(b.tags[j]); }
                if printed { prints.push(b.prints[j]); }
                j += 1;
            }
        }
        ids.extend_from_slice(&a.ids[i..]);
        ids.extend_from_slice(&b.ids[j..]);
        if printed {
            prints.extend_from_slice(&a.prints[i..]);
            prints.extend_from_slice(&b.prints[j..]);
        }
        if tagged {
            tags.extend_from_slice(&a.tags[i..]);
            tags.extend_from_slice(&b.tags[j..]);
            return Run { ids, fences: Vec::new(), tags, prints };
        }
        Run { prints, ..Run::fenced(data, k, cols, ids) }
    }

    /// Calls `f` on each group of ids equal on the first `m` columns of `cols`, in order.
    fn each_group<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], m: usize, mut f: impl FnMut(&[u32])) {
        let same = |a: u32, b: u32| cols[..m].iter().all(|&c| data.row(a, k)[c] == data.row(b, k)[c]);
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

/// `ids` sorted as `cmp` orders them, where `cmp` orders by the leading value's tag (`tagged`) and then the order's
/// columns in turn, `val(id, i)` being an id's value in the order's column `i` (of `ncols`): the ids are sorted with
/// their tag and first two values held beside them, read once a row, and rows are compared (`cmp`) only within a
/// stretch equal in all three. The sorted ids' tags are returned (empty when untagged). Held beside the ids, the keys
/// are 24 bytes a row sorted, for the sort only; compared through the rows instead, each comparison read two
/// scattered rows (O125: 8 G instructions of the cold run's 346 G).
///
/// Packed (decision 0179): the tag, the first two values less their least, and the id, each in as few bits as its
/// range needs, side by side in one u64 (8 bytes a row) or u128 (16) when they fit; the 24-byte tuple otherwise.
/// Each field is ordered as its value, so the packed keys sort as the tuples do. `RERAIL_SORT_PACKED=off` keeps
/// the tuple (a measurement aid).
pub(super) fn sort_keyed(ids: &mut [u32], tagged: bool, ncols: usize, val: impl Fn(u32, usize) -> i64, cmp: impl Fn(u32, u32) -> Ordering) -> Vec<u16> {
    static PACKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *PACKED.get_or_init(|| std::env::var("RERAIL_SORT_PACKED").map_or(true, |v| v != "off"))
        && let Some(tags) = sort_packed(ids, tagged, ncols, &val, &cmp)
    {
        return tags;
    }
    sort_tupled(ids, tagged, ncols, val, cmp)
}

/// `sort_keyed` by packed keys, or `None` (nothing done) when its fields need more than 128 bits.
fn sort_packed(ids: &mut [u32], tagged: bool, ncols: usize, val: &impl Fn(u32, usize) -> i64, cmp: &impl Fn(u32, u32) -> Ordering) -> Option<Vec<u16>> {
    let (n, nv) = (ids.len(), ncols.min(2));
    // The held fields of each row, read once: its tag (when tagged), then its first two values, each value's bits
    // with the sign flipped, so the fields order as the signed values do.
    let nf = usize::from(tagged) + nv;
    if n < 2 || nf == 0 {
        return None;
    }
    let flip = |v: i64| (v as u64) ^ (1 << 63);
    let mut f: Vec<u64> = Vec::with_capacity(n * nf);
    let mut top = 0u32;
    for &id in ids.iter() {
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
    let bits = |span: u64| 64 - span.leading_zeros();
    let wid = bits(u64::from(top));
    if nf == 1 {
        // One value (most orders): packed in place, `(v - least) << id bits | id`.
        let (lo, hi) = f.iter().fold((u64::MAX, 0), |(l, h), &v| (l.min(v), h.max(v)));
        if bits(hi - lo) + wid > 64 {
            return None;
        }
        for (k, &id) in f.iter_mut().zip(ids.iter()) {
            *k = (*k - lo) << wid | u64::from(id);
        }
        f.sort_unstable();
        let mask = (1u64 << wid) - 1;
        for (slot, k) in ids.iter_mut().zip(&f) {
            *slot = (k & mask) as u32;
        }
        return Some(Vec::new());
    }
    let (mut lo, mut hi) = ([u64::MAX; 3], [0u64; 3]);
    for row in f.chunks_exact(nf) {
        for (j, &v) in row.iter().enumerate() {
            lo[j] = lo[j].min(v);
            hi[j] = hi[j].max(v);
        }
    }
    let mut w = [0u32; 3];
    for j in 0..nf {
        w[j] = bits(hi[j] - lo[j]);
    }
    let total = w.iter().sum::<u32>() + wid;
    if total > 128 {
        return None;
    }
    // Each field's place: the first highest, the id lowest.
    let mut shift = [0u32; 3];
    let mut at = wid;
    for j in (0..nf).rev() {
        shift[j] = at;
        at += w[j];
    }
    // Zero-width fields hold nothing (their shift may be the key's whole width).
    let (sh, lo_) = (shift, lo);
    let pack = |row: &[u64], id: u32| -> u128 {
        let mut k = u128::from(id);
        for j in 0..row.len() {
            if w[j] > 0 {
                k |= u128::from(row[j] - lo_[j]) << sh[j];
            }
        }
        k
    };
    let mask = (1u128 << wid) - 1;
    let keys: Vec<u128> = if total <= 64 {
        let mut narrow: Vec<u64> = f.chunks_exact(nf).zip(ids.iter()).map(|(row, &id)| pack(row, id) as u64).collect();
        drop(f);
        narrow.sort_unstable();
        for (slot, &k) in ids.iter_mut().zip(&narrow) {
            *slot = (u128::from(k) & mask) as u32;
        }
        if ncols <= 2 && !tagged {
            return Some(Vec::new());
        }
        narrow.into_iter().map(u128::from).collect()
    } else {
        let mut keys: Vec<u128> = f.chunks_exact(nf).zip(ids.iter()).map(|(row, &id)| pack(row, id)).collect();
        drop(f);
        keys.sort_unstable();
        for (slot, k) in ids.iter_mut().zip(&keys) {
            *slot = (k & mask) as u32;
        }
        keys
    };
    // The stretches equal in every held field, which `cmp` orders when the order has more columns.
    if ncols > 2 {
        let mut at = 0;
        while at < keys.len() {
            let mut end = at + 1;
            while end < keys.len() && keys[end] >> wid == keys[at] >> wid {
                end += 1;
            }
            if end - at > 1 {
                ids[at..end].sort_unstable_by(|&a, &b| cmp(a, b));
            }
            at = end;
        }
    }
    Some(if tagged { keys.iter().map(|&k| ((k >> shift[0]) as u64 + lo[0]) as u16).collect() } else { Vec::new() })
}

/// `sort_keyed` by 24-byte tuples of the tag, the first two values and the id.
fn sort_tupled(ids: &mut [u32], tagged: bool, ncols: usize, val: impl Fn(u32, usize) -> i64, cmp: impl Fn(u32, u32) -> Ordering) -> Vec<u16> {
    let mut keys: Vec<(u16, i64, i64, u32)> = ids
        .iter()
        .map(|&id| {
            let v0 = if ncols > 0 { val(id, 0) } else { 0 };
            (if tagged { tag(v0) } else { 0 }, v0, if ncols > 1 { val(id, 1) } else { 0 }, id)
        })
        .collect();
    keys.sort_unstable();
    for (slot, key) in ids.iter_mut().zip(&keys) {
        *slot = key.3;
    }
    if ncols > 2 {
        let mut at = 0;
        while at < keys.len() {
            let mut end = at + 1;
            while end < keys.len() && keys[end].0 == keys[at].0 && keys[end].1 == keys[at].1 && keys[end].2 == keys[at].2 {
                end += 1;
            }
            if end - at > 1 {
                ids[at..end].sort_unstable_by(|&a, &b| cmp(a, b));
            }
            at = end;
        }
    }
    if tagged { keys.iter().map(|k| k.0).collect() } else { Vec::new() }
}

impl Index {
    pub(super) fn new(cols: Vec<usize>) -> Index {
        Index { cols, runs: Vec::new(), rent: AtomicU64::new(0), printed: false }
    }

    /// An order its probes give whole keys to (a derived input's own index, decision 0112): a byte a row more, each
    /// id's key print, so a probe reads the prints of its window, not its rows (`print_span`).
    pub(super) fn printed(cols: Vec<usize>) -> Index {
        Index { printed: true, ..Index::new(cols) }
    }

    /// A run of the rows `lo..hi` in this order, printed if the order is: the prints made in row order (the rows
    /// read in turn), then placed as the ids sort.
    fn run_of<D: RowData + ?Sized>(&self, data: &D, k: usize, lo: u32, hi: u32) -> Run {
        presorted::note(data, k, &self.cols, lo, hi, self.printed);
        let _t = presorted::Timed::start(self.printed);
        if !self.printed {
            return Run::of(data, k, &self.cols, (lo..hi).collect());
        }
        let by_row: Vec<u8> = (lo..hi).map(|id| print_of(data.row(id, k), &self.cols)).collect();
        let mut run = Run::of(data, k, &self.cols, (lo..hi).collect());
        run.prints = run.ids.iter().map(|&id| by_row[(id - lo) as usize]).collect();
        run
    }

    /// Whether the set `keys` is a prefix of this order's columns (in any order).
    pub(super) fn answers(&self, keys: &[usize]) -> bool {
        !keys.is_empty() && keys.len() <= self.cols.len() && keys.iter().all(|c| self.cols[..keys.len()].contains(c))
    }

    /// The rows `lo..hi` as a new run of wave `wave` (dead rows are indexed as any; readers skip them).
    pub(super) fn add_run<D: RowData + ?Sized>(&mut self, data: &D, k: usize, lo: usize, hi: usize, wave: u32) {
        if lo < hi {
            let run = self.run_of(data, k, lo as u32, hi as u32);
            self.runs.push((Arc::new(run), wave));
        }
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
        // Pairwise, smallest first: each id is copied about log(n) times.
        let mut parts: Vec<Arc<Run>> = self.runs[..n].iter().map(|(r, _)| Arc::clone(r)).collect();
        while parts.len() > 1 {
            parts.sort_by_key(|r| std::cmp::Reverse(r.ids.len()));
            let a = parts.pop().expect("a run");
            let b = parts.pop().expect("a run");
            parts.push(Arc::new(Run::merge(data, k, &self.cols, &a, &b)));
        }
        Arc::try_unwrap(parts.pop().expect("a run")).unwrap_or_else(|r| Run { ids: r.ids.clone(), fences: r.fences.clone(), tags: r.tags.clone(), prints: r.prints.clone() })
    }

    /// At most `max` runs, the two smallest merged at a time (a relation's frozen order: its probes cannot merge it
    /// later, and a large run is left alone while the small ones grow to its size).
    pub(super) fn tier<D: RowData + ?Sized>(&mut self, data: &D, k: usize, max: usize) {
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
    pub(super) fn reorder<D: RowData + ?Sized>(&mut self, data: &D, k: usize, cols: Vec<usize>) {
        if cols == self.cols {
            return;
        }
        self.cols = cols;
        for (r, _) in &mut self.runs {
            let run = Run::of(data, k, &self.cols, r.ids.clone());
            *r = Arc::new(if self.printed { run.printed(data, k, &self.cols) } else { run });
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
    pub(super) fn find<'a, D: RowData + ?Sized>(&'a self, data: &D, k: usize, key: &[i64], delta: Option<u32>) -> Found<'a> {
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
    pub(super) fn find_at<'a, D: RowData + ?Sized>(&'a self, data: &D, k: usize, key: &[i64], delta: Option<u32>, hints: &mut [u32; HINTS]) -> Found<'a> {
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
    pub(super) fn held_sorted<D: RowData + ?Sized>(&self, data: &D, k: usize, rows: &[i64], order: &[u32], live: impl Fn(u32) -> bool, held: &mut [bool]) {
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
    pub(super) fn each_group<D: RowData + ?Sized>(&self, data: &D, k: usize, m: usize, f: impl FnMut(&[u32])) {
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
        self.runs.iter().map(|(r, _)| r.ids.capacity() * 4 + r.tags.capacity() * 2 + r.prints.capacity() + r.fences.capacity() * 8 + 64).sum::<usize>() + self.runs.capacity() * 16
    }
}

/// Measurement aid (`RERAIL_SPIKE_PRESORTED`): how much of each run's rows come already in the order's key order
/// (row id order), the case known-sorted ranges would skip the sort for.
pub mod presorted {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    /// [derived input orders, component orders] × [runs, rows, runs wholly in order, rows in stretches of 64 or more
    /// in order, stretches of 64 or more].
    static COUNTS: [[AtomicU64; 6]; 2] = [const { [const { AtomicU64::new(0) }; 6] }; 2];
    /// A run's build time (ns), into `COUNTS[_][5]`, when measuring.
    pub(super) struct Timed(Option<(std::time::Instant, bool)>);
    impl Timed {
        pub(super) fn start(printed: bool) -> Timed {
            Timed(std::env::var_os("RERAIL_SPIKE_PRESORTED").is_some().then(|| (std::time::Instant::now(), printed)))
        }
    }
    impl Drop for Timed {
        fn drop(&mut self) {
            if let Some((t, printed)) = self.0 {
                COUNTS[usize::from(!printed)][5].fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
            }
        }
    }
    pub(super) fn note<D: RowData + ?Sized>(data: &D, k: usize, cols: &[usize], lo: u32, hi: u32, printed: bool) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| std::env::var_os("RERAIL_SPIKE_PRESORTED").is_some()) || hi <= lo {
            return;
        }
        let c = &COUNTS[usize::from(!printed)];
        let (mut start, mut long, mut stretches, mut whole) = (lo, 0u64, 0u64, true);
        let mut close = |start: u32, end: u32| if end - start >= 64 { long += u64::from(end - start); stretches += 1; };
        for id in lo + 1..hi {
            if cmp_vals(cols, tagged(cols, k), data.row(id - 1, k), data.row(id, k)) == Ordering::Greater {
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
    pub fn report() -> String {
        let row = |i: usize| COUNTS[i].iter().map(|a| a.load(Relaxed)).collect::<Vec<_>>();
        let (d, o) = (row(0), row(1));
        let f = |v: &[u64]| format!("{} runs ({} wholly in order), {} rows, {} in order in {} stretches of 64+ ({:.0}%); built in {:.1} ms", v[0], v[2], v[1], v[3], v[4], v[3] as f64 * 100.0 / v[1].max(1) as f64, v[5] as f64 / 1e6);
        format!("derived-input orders: {}; component orders: {}", f(&d), f(&o))
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

    /// Packed keys (decision 0179) sort as the 24-byte tuples do, ids and tags alike: values narrow (a u64), wide or
    /// negative (a u128), node ids (`d << 32 | x`), and too wide to pack (the tuples); one, two and three columns,
    /// tagged (every column) and not; keys equal in their first two values (left to `cmp`) and repeated rows (by id).
    #[test]
    fn packed_keys_sort_as_the_tuples_do() {
        let k = 3;
        let n = 3000i64;
        let shapes: [(&str, fn(i64) -> i64); 5] = [
            ("narrow", |i| i % 37),
            ("negative", |i| (i % 211) - 105),
            ("node", |i| ((i % 9) << 32) | (i * 7) % 4001),
            ("wide", |i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64)),
            ("extremes", |i| [i64::MIN, -1, 0, 1, i64::MAX][i as usize % 5]),
        ];
        let mut packed_seen = [false; 2];
        for (name, f) in shapes {
            let rows: Vec<i64> = (0..n).flat_map(|i| [f(i), f(i / 3), i % 5]).collect();
            for (cols, tagged) in [(vec![0], false), (vec![0, 1], false), (vec![1, 0], false), (vec![0, 1, 2], true)] {
                let val = |id: u32, i: usize| rows[id as usize * k + cols[i]];
                let cmp = |a: u32, b: u32| cmp_rows(&rows[..], k, &cols, a, b);
                let start: Vec<u32> = (0..n as u32).rev().collect();
                let (mut a, mut b) = (start.clone(), start);
                let ta = sort_packed(&mut a, tagged, cols.len(), &val, &cmp);
                let tb = sort_tupled(&mut b, tagged, cols.len(), val, cmp);
                packed_seen[usize::from(name == "wide")] |= ta.is_some();
                let Some(ta) = ta else { continue };
                assert_eq!(a, b, "{name} {cols:?} tagged={tagged}");
                assert_eq!(ta, tb, "{name} {cols:?} tagged={tagged}");
            }
        }
        assert!(packed_seen[0], "the narrow shapes pack");
    }

    /// Measurement aid (decision 0179): instructions and cycles a row of the packed and tupled sorts, over orders of
    /// one and two columns shaped as the O125 replay's large derived-input builds (node ids, `file << 32 | at`).
    #[test]
    #[ignore]
    fn sort_keyed_costs() {
        let n = 50_000u32;
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        for shape in ["clustered", "shuffled", "small"] {
            let rows: Vec<i64> = (0..n as i64).flat_map(|i| {
                let node = match shape { "clustered" => ((i / 600) << 32) | (i % 600) * 3, "shuffled" => (((rand() % 9000) as i64) << 32) | (rand() % 4000) as i64, _ => (rand() % 40_000) as i64 };
                [node, (rand() % 5000) as i64]
            }).collect();
            for cols in [vec![0usize], vec![0, 1]] {
                let k = 2;
                let val = |id: u32, i: usize| rows[id as usize * k + cols[i]];
                let cmp = |a: u32, b: u32| cmp_rows(&rows[..], k, &cols, a, b);
                let mut cost = |packed: bool| {
                    let (mut ins, mut cyc) = (u64::MAX, u64::MAX);
                    for _ in 0..5 {
                        let mut ids: Vec<u32> = (0..n).collect();
                        let (i0, c0) = rerail_compiler::native_cpu_codegen::retired();
                        if packed { std::hint::black_box(sort_packed(&mut ids, false, cols.len(), &val, &cmp)); } else { std::hint::black_box(sort_tupled(&mut ids, false, cols.len(), val, cmp)); }
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
        let rows: Vec<i64> = (0..n).flat_map(|i| [if i % 2 == 0 { i % 3 } else { (i * 7919) % 2003 }, (i * 31) % 97, i % 1009]).collect();
        let ids = |f: Found| { let mut v: Vec<u32> = f.parts().flat_map(|s| s.iter().copied()).collect(); v.sort_unstable(); v };
        for cols in [vec![0], vec![0, 1], vec![0, 1, 2]] {
            let (mut plain, mut printed) = (Index::new(cols.clone()), Index::printed(cols.clone()));
            for (w, (lo, hi)) in [(0, 2500), (2500, 4000), (4000, 6000)].into_iter().enumerate() {
                plain.add_run(&rows, k, lo, hi, w as u32 + 1);
                printed.add_run(&rows, k, lo, hi, w as u32 + 1);
            }
            // Keys: every row's, and as many absent (values no row holds in some column).
            let mut keys: Vec<Vec<i64>> = (0..n as usize).map(|r| cols.iter().map(|&c| rows[r * k + c]).collect()).collect();
            keys.extend((0..n).map(|i| cols.iter().enumerate().map(|(j, _)| if j + 1 == cols.len() { 5000 + i } else { i % 3 }).collect()));
            let check = |plain: &Index, printed: &Index, keys: &[Vec<i64>]| {
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
            assert_eq!(printed.bytes(), plain.bytes() + printed.rows(), "a byte a row more");
        }
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
