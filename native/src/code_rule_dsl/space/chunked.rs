//! Node sets as chunked containers (decision 0045).
//!
//! A node is its number in the space (`NodeSpace::base(file) + position`). The space is cut into
//! chunks of 2^16 nodes, across files as in Roaring; each chunk that holds a member is one
//! container, found in a sorted directory by `node >> 16`:
//!
//! - **array**: sorted low 16 bits;
//! - **bitmap**: `u64` words up to the last member (at most 1,024: 8 KB);
//! - **runs**: sorted inclusive `(start, last)` intervals.
//!
//! Every container lives in one of the set's two buffers (`values` for arrays and runs, `words` for
//! bitmaps); the directory holds (key, form, offset, length). An operation walks the directories and
//! writes its result's containers straight into the result's buffers, so it allocates per set, not
//! per chunk; and the directory is the device's work list as it stands (§4).
//!
//! A chunk is a bitmap once it holds more members than its bitmap has words (a density over 1/64):
//! word operations then beat per-member work. Below that it is an array, or runs where those are
//! smaller. A bitmap is priced at the words up to its last member, not at 2^16 bits as in Roaring.
//!
//! A chunk holds about 32 files of the 9,010-file repository (2,045 nodes on average), so the
//! per-chunk work of an operation is paid 282 times there, not once per file (0045, profile).
//! Files are ranges of the space: per-file questions (`within_files`, `file_nodes`, a file's
//! stored segment) take the space. Sets are built in node order by `Builder`.

use std::cmp::Ordering;

use super::NodeSpace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Array,
    Bitmap,
    Runs,
}

/// One chunk: its key, its form, and where its container is in the set's buffers (`len` counts
/// values, words or runs).
#[derive(Clone, Copy, Debug)]
struct Entry {
    key: u64,
    form: Form,
    at: u32,
    len: u32,
}

/// A chunk's container, borrowed from a set's buffers.
#[derive(Clone, Copy)]
enum View<'a> {
    Array(&'a [u16]),
    Bitmap(&'a [u64]),
    Runs(&'a [(u16, u16)]),
}

/// A set of a space's nodes, as chunked containers over the space's node numbers.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct NodeSet {
    directory: Vec<Entry>,
    values: Vec<u16>,
    words: Vec<u64>,
}

impl PartialEq for NodeSet {
    /// Equal members, whatever the containers.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for NodeSet {}

/// A node's directory key (its 2^16 slice of the space) and its value in the chunk.
fn key(node: u32) -> (u64, u16) {
    (u64::from(node >> 16), node as u16)
}

fn words_to(last: u16) -> usize {
    usize::from(last) / 64 + 1
}

fn runs_of(values: &[u16]) -> &[(u16, u16)] {
    // SAFETY: `(u16, u16)` is two `u16`s with no padding, align 2; `values` holds pairs.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), values.len() / 2) }
}

impl View<'_> {
    fn len(self) -> usize {
        match self {
            View::Array(a) => a.len(),
            View::Bitmap(b) => b.iter().map(|w| w.count_ones() as usize).sum(),
            View::Runs(r) => r.iter().map(|&(s, l)| usize::from(l - s) + 1).sum(),
        }
    }

    fn contains(self, v: u16) -> bool {
        match self {
            View::Array(a) => a.binary_search(&v).is_ok(),
            View::Bitmap(b) => b.get(usize::from(v) / 64).is_some_and(|w| w >> (v % 64) & 1 == 1),
            View::Runs(r) => r.binary_search_by(|&(s, l)| if l < v { Ordering::Less } else if s > v { Ordering::Greater } else { Ordering::Equal }).is_ok(),
        }
    }

    fn last(self) -> u16 {
        match self {
            View::Array(a) => *a.last().expect("a container is not empty"),
            View::Runs(r) => r.last().expect("a container is not empty").1,
            View::Bitmap(b) => (b.len() * 64 - 1 - b.last().expect("a container is not empty").leading_zeros() as usize) as u16,
        }
    }

    /// Ors the members into `out`, which is long enough.
    fn or_into(self, out: &mut [u64]) {
        match self {
            View::Array(a) => a.iter().for_each(|&v| out[usize::from(v) / 64] |= 1 << (v % 64)),
            View::Runs(r) => r.iter().for_each(|&(s, l)| fill(out, s, l)),
            View::Bitmap(b) => out.iter_mut().zip(b).for_each(|(o, w)| *o |= w),
        }
    }

    /// Whether a member is in `a..=b`.
    fn any_between(self, a: u16, b: u16) -> bool {
        match self {
            View::Array(x) => x.get(x.partition_point(|&v| v < a)).is_some_and(|&v| v <= b),
            View::Runs(r) => r.get(r.partition_point(|&(_, l)| l < a)).is_some_and(|&(s, _)| s <= b),
            View::Bitmap(w) => {
                let (first, last) = (usize::from(a) / 64, (usize::from(b) / 64).min(w.len().wrapping_sub(1)));
                if w.is_empty() || first > last {
                    return false;
                }
                let lo = !0u64 << (a % 64);
                let hi = if usize::from(b) / 64 == last { !0u64 >> (63 - b % 64) } else { !0 };
                if first == last {
                    return w[first] & lo & hi != 0;
                }
                w[first] & lo != 0 || w[first + 1..last].iter().any(|&x| x != 0) || w[last] & hi != 0
            }
        }
    }

    fn for_each(self, mut f: impl FnMut(u16)) {
        match self {
            View::Array(a) => a.iter().for_each(|&v| f(v)),
            View::Runs(r) => r.iter().for_each(|&(s, l)| (s..=l).for_each(&mut f)),
            View::Bitmap(b) => bits(b).for_each(f),
        }
    }
}

/// Members in `words`. Four accumulators, so the counts vectorize.
fn count(words: &[u64]) -> usize {
    let mut acc = [0u64; 4];
    let mut chunks = words.chunks_exact(4);
    for c in &mut chunks {
        for i in 0..4 {
            acc[i] += u64::from(c[i].count_ones());
        }
    }
    (acc.iter().sum::<u64>() + chunks.remainder().iter().map(|w| u64::from(w.count_ones())).sum::<u64>()) as usize
}

/// `out &= other` (equal lengths), returning the members left.
fn and_count(out: &mut [u64], other: &[u64]) -> usize {
    out.iter_mut().zip(other).for_each(|(o, b)| *o &= b);
    count(out)
}

/// `out &= !other` (equal lengths), returning the members left.
fn and_not_count(out: &mut [u64], other: &[u64]) -> usize {
    out.iter_mut().zip(other).for_each(|(o, b)| *o &= !b);
    count(out)
}

/// `out |= other` (equal lengths), returning the members.
fn or_count(out: &mut [u64], other: &[u64]) -> usize {
    out.iter_mut().zip(other).for_each(|(o, b)| *o |= b);
    count(out)
}

/// Appends the members of `words` (`n` of them), plus `offset`, to `out`: one tight loop.
fn push_bits(words: &[u64], offset: usize, n: usize, out: &mut Vec<u16>) {
    out.reserve(n);
    for (at, &w) in words.iter().enumerate() {
        let mut w = w;
        let base = (offset + at * 64) as u16;
        while w != 0 {
            out.push(base + w.trailing_zeros() as u16);
            w &= w - 1;
        }
    }
}

/// `bytes` as little-endian `u16`s, into `out`: one copy on a little-endian host.
fn le_u16s(bytes: &[u8], out: &mut Vec<u16>) {
    let n = bytes.len() / 2;
    out.clear();
    out.reserve(n);
    // SAFETY: `out` has room for `n` u16s (2n bytes), `bytes` holds at least 2n bytes, they do
    // not overlap, and every bit pattern is a valid u16; the copy initializes the first `n`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr().cast::<u8>(), 2 * n);
        out.set_len(n);
    }
    if cfg!(target_endian = "big") {
        out.iter_mut().for_each(|v| *v = u16::from_le(*v));
    }
}

/// `bytes` as little-endian `u64`s, into `out`.
fn le_u64s(bytes: &[u8], out: &mut Vec<u64>) {
    let n = bytes.len() / 8;
    out.clear();
    out.reserve(n);
    // SAFETY: `out` has room for `n` u64s (8n bytes), `bytes` holds at least 8n bytes, they do
    // not overlap, and every bit pattern is a valid u64; the copy initializes the first `n`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr().cast::<u8>(), 8 * n);
        out.set_len(n);
    }
    if cfg!(target_endian = "big") {
        out.iter_mut().for_each(|v| *v = u64::from_le(*v));
    }
}

/// Bit `i` of the result is the xor of `w`'s bits `0..=i`: a carry-less multiply by all ones.
#[inline]
fn prefix_xor(w: u64) -> u64 {
    #[cfg(all(target_arch = "aarch64", target_feature = "aes"))]
    {
        // SAFETY: the `aes` target feature (PMULL) is enabled for this build.
        unsafe { std::arch::aarch64::vmull_p64(w, !0) as u64 }
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "aes")))]
    {
        let mut w = w;
        for shift in [1, 2, 4, 8, 16, 32] {
            w ^= w << shift;
        }
        w
    }
}

/// Clears bits `s..=l`, cut at the bitmap's end.
fn clear_clipped(b: &mut [u64], s: u16, l: u16) {
    let end = b.len() * 64;
    if usize::from(s) >= end {
        return;
    }
    let (s, l) = (usize::from(s), usize::from(l).min(end - 1));
    let (ws, wl) = (s / 64, l / 64);
    let lo = !0u64 << (s % 64);
    let hi = !0u64 >> (63 - l % 64);
    if ws == wl {
        b[ws] &= !(lo & hi);
    } else {
        b[ws] &= !lo;
        b[ws + 1..wl].iter_mut().for_each(|w| *w = 0);
        b[wl] &= !hi;
    }
}

/// Sets bits `s..=l`, the bitmap long enough.
fn fill(b: &mut [u64], s: u16, l: u16) {
    let (s, l) = (usize::from(s), usize::from(l));
    let (ws, wl) = (s / 64, l / 64);
    let lo = !0u64 << (s % 64);
    let hi = !0u64 >> (63 - l % 64);
    if ws == wl {
        b[ws] |= lo & hi;
    } else {
        b[ws] |= lo;
        b[ws + 1..wl].iter_mut().for_each(|w| *w = !0);
        b[wl] |= hi;
    }
}

fn bits(b: &[u64]) -> impl Iterator<Item = u16> + '_ {
    b.iter().enumerate().flat_map(|(at, &w)| {
        let mut w = w;
        std::iter::from_fn(move || {
            (w != 0).then(|| {
                let bit = w.trailing_zeros();
                w &= w - 1;
                (at * 64) as u16 + bit as u16
            })
        })
    })
}

/// The array's values inside (or outside, `inside == false`) the runs: one walk of both.
fn filter_by_runs(a: &[u16], r: &[(u16, u16)], inside: bool, out: &mut Vec<u16>) {
    let mut j = 0;
    for &v in a {
        while j < r.len() && r[j].1 < v {
            j += 1;
        }
        if (j < r.len() && r[j].0 <= v) == inside {
            out.push(v);
        }
    }
}

/// Galloping when one side is much shorter, a merge otherwise.
fn intersect_sorted(x: &[u16], y: &[u16], out: &mut Vec<u16>) {
    let (small, large) = if x.len() <= y.len() { (x, y) } else { (y, x) };
    if small.len() * 16 < large.len() {
        let mut from = 0;
        for &v in small {
            match large[from..].binary_search(&v) {
                Ok(k) => {
                    out.push(v);
                    from += k + 1;
                }
                Err(k) => from += k,
            }
        }
        return;
    }
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        match x[i].cmp(&y[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                out.push(x[i]);
                i += 1;
                j += 1;
            }
        }
    }
}

fn union_sorted(x: &[u16], y: &[u16], out: &mut Vec<u16>) {
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        match x[i].cmp(&y[j]) {
            Ordering::Less => {
                out.push(x[i]);
                i += 1;
            }
            Ordering::Greater => {
                out.push(y[j]);
                j += 1;
            }
            Ordering::Equal => {
                out.push(x[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&x[i..]);
    out.extend_from_slice(&y[j..]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    And,
    Or,
    AndNot,
}

impl NodeSet {
    pub(in crate::code_rule_dsl) fn empty(_space: &NodeSpace) -> Self {
        Self::default()
    }

    /// Every node of the space: one run per chunk.
    pub(in crate::code_rule_dsl) fn all(space: &NodeSpace) -> Self {
        let mut b = Builder::default();
        b.run(0, space.len());
        b.finish()
    }

    fn view(&self, e: &Entry) -> View<'_> {
        let (at, len) = (e.at as usize, e.len as usize);
        match e.form {
            Form::Array => View::Array(&self.values[at..at + len]),
            Form::Runs => View::Runs(runs_of(&self.values[at..at + 2 * len])),
            Form::Bitmap => View::Bitmap(&self.words[at..at + len]),
        }
    }

    /// File `file`'s positions (ascending), added to the set.
    #[cfg_attr(not(test), allow(dead_code))] // tests and measurements
    pub(in crate::code_rule_dsl) fn insert_file(&mut self, space: &NodeSpace, file: usize, positions: impl IntoIterator<Item = u32>) {
        let base = space.base(file);
        let mut b = Builder::default();
        positions.into_iter().for_each(|p| b.node(base + p));
        self.union_with(&b.finish());
    }

    /// File `file`'s runs `ranges` (sorted, apart), added to the set: §3's tree axes build sets this way.
    #[allow(dead_code)] // no tree-axis set is built on the host yet
    pub(in crate::code_rule_dsl) fn insert_runs(&mut self, space: &NodeSpace, file: usize, ranges: impl IntoIterator<Item = std::ops::Range<u32>>) {
        let base = space.base(file);
        let mut b = Builder::default();
        ranges.into_iter().for_each(|r| b.run(base + r.start, base + r.end));
        self.union_with(&b.finish());
    }

    /// Adds every node of file `file`.
    #[cfg_attr(not(test), allow(dead_code))] // tests and measurements
    pub(in crate::code_rule_dsl) fn fill_file(&mut self, space: &NodeSpace, file: usize) {
        let mut b = Builder::default();
        let range = space.range(file);
        b.run(range.start, range.end);
        self.union_with(&b.finish());
    }

    /// The set of `words`, a slot of the space's dense words (Metal's `set_steps`).
    pub(in crate::code_rule_dsl) fn from_words(_space: &NodeSpace, words: &[u64]) -> Self {
        let mut b = Builder::default();
        b.words_at(0, words);
        b.finish()
    }

    /// The set as the space's dense words (Metal's `set_steps`).
    pub(in crate::code_rule_dsl) fn to_words(&self, space: &NodeSpace) -> Vec<u64> {
        let mut words = vec![0_u64; space.words()];
        for e in &self.directory {
            let at = (e.key as usize) << 10;
            let end = words.len().min(at + 1024);
            self.view(e).or_into_clipped(&mut words[at..end]);
        }
        words
    }

    /// The array in `values[from..]` is chunk `k`: kept, or moved to the form that suits it.
    fn seal_array(&mut self, k: u64, from: usize) {
        debug_assert!(self.directory.last().is_none_or(|e| e.key < k), "chunks are added in order");
        let n = self.values.len() - from;
        if n == 0 {
            return;
        }
        let last = self.values[self.values.len() - 1];
        let w = words_to(last);
        let runs = 1 + self.values[from..].windows(2).filter(|p| p[1] != p[0] + 1).count();
        if 4 * runs < (2 * n).min(8 * w) {
            let mut out: Vec<u16> = Vec::with_capacity(2 * runs);
            for &v in &self.values[from..] {
                match out.len() {
                    0 => out.extend([v, v]),
                    len if out[len - 1].checked_add(1) == Some(v) => out[len - 1] = v,
                    _ => out.extend([v, v]),
                }
            }
            self.values.truncate(from);
            self.values.extend_from_slice(&out);
            self.directory.push(Entry { key: k, form: Form::Runs, at: from as u32, len: runs as u32 });
            return;
        }
        if n > w {
            let mut b = vec![0u64; w];
            View::Array(&self.values[from..]).or_into(&mut b);
            self.values.truncate(from);
            self.push_bitmap(k, b);
            return;
        }
        self.directory.push(Entry { key: k, form: Form::Array, at: from as u32, len: n as u32 });
    }

    /// The runs in `values[from..]` are chunk `k`.
    fn seal_runs(&mut self, k: u64, from: usize) {
        debug_assert!(self.directory.last().is_none_or(|e| e.key < k), "chunks are added in order");
        let runs = (self.values.len() - from) / 2;
        if runs > 0 {
            self.directory.push(Entry { key: k, form: Form::Runs, at: from as u32, len: runs as u32 });
        }
    }

    /// Chunk `k` as bitmap words: trimmed, and as an array when that suits it better.
    fn push_bitmap(&mut self, k: u64, mut b: Vec<u64>) {
        while b.last() == Some(&0) {
            b.pop();
        }
        if b.is_empty() {
            return;
        }
        let n: usize = b.iter().map(|w| w.count_ones() as usize).sum();
        if n <= b.len() {
            let from = self.values.len();
            push_bits(&b, 0, n, &mut self.values);
            self.directory.push(Entry { key: k, form: Form::Array, at: from as u32, len: n as u32 });
            return;
        }
        self.directory.push(Entry { key: k, form: Form::Bitmap, at: self.words.len() as u32, len: b.len() as u32 });
        self.words.extend_from_slice(&b);
    }

    /// Bitmap words of chunk `k` written in place at `words[from..]`: trimmed, and kept or moved.
    /// `seal_words` with the members already counted.
    fn seal_counted(&mut self, k: u64, from: usize, n: usize) {
        if n == 0 {
            self.words.truncate(from);
            return;
        }
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
        let len = self.words.len() - from;
        if n <= len {
            let at = self.values.len();
            push_bits(&self.words[from..], 0, n, &mut self.values);
            self.words.truncate(from);
            self.directory.push(Entry { key: k, form: Form::Array, at: at as u32, len: n as u32 });
            return;
        }
        self.directory.push(Entry { key: k, form: Form::Bitmap, at: from as u32, len: len as u32 });
    }

    fn seal_words(&mut self, k: u64, from: usize) {
        while self.words.len() > from && self.words.last() == Some(&0) {
            self.words.pop();
        }
        let len = self.words.len() - from;
        if len == 0 {
            return;
        }
        let n: usize = self.words[from..].iter().map(|w| w.count_ones() as usize).sum();
        if n <= len {
            let at = self.values.len();
            push_bits(&self.words[from..], 0, n, &mut self.values);
            self.words.truncate(from);
            self.directory.push(Entry { key: k, form: Form::Array, at: at as u32, len: n as u32 });
            return;
        }
        self.directory.push(Entry { key: k, form: Form::Bitmap, at: from as u32, len: len as u32 });
    }

    /// Copies a chunk of `other` into the set.
    fn copy_chunk(&mut self, other: &Self, e: &Entry) {
        let (at, len) = (e.at as usize, e.len as usize);
        let mut copy = *e;
        match e.form {
            Form::Array => {
                copy.at = self.values.len() as u32;
                self.values.extend_from_slice(&other.values[at..at + len]);
            }
            Form::Runs => {
                copy.at = self.values.len() as u32;
                self.values.extend_from_slice(&other.values[at..at + 2 * len]);
            }
            Form::Bitmap => {
                copy.at = self.words.len() as u32;
                self.words.extend_from_slice(&other.words[at..at + len]);
            }
        }
        self.directory.push(copy);
    }

    /// Chunk `k` of `op` on two containers, written into the set.
    fn combine(&mut self, k: u64, op: Op, a: View<'_>, b: View<'_>) {
        use View::*;
        let from = self.values.len();
        match (op, a, b) {
            (Op::And, Array(x), Array(y)) => {
                intersect_sorted(x, y, &mut self.values);
                self.seal_array(k, from)
            }
            (Op::Or, Array(x), Array(y)) => {
                union_sorted(x, y, &mut self.values);
                self.seal_array(k, from)
            }
            (Op::And, Array(x), Runs(r)) | (Op::And, Runs(r), Array(x)) => {
                filter_by_runs(x, r, true, &mut self.values);
                self.seal_array(k, from)
            }
            (Op::AndNot, Array(x), Runs(r)) => {
                filter_by_runs(x, r, false, &mut self.values);
                self.seal_array(k, from)
            }
            (Op::And, Array(x), m @ Bitmap(_)) | (Op::And, m @ Bitmap(_), Array(x)) => {
                self.values.extend(x.iter().copied().filter(|&v| m.contains(v)));
                self.seal_array(k, from)
            }
            (Op::AndNot, Array(x), m) => {
                self.values.extend(x.iter().copied().filter(|&v| !m.contains(v)));
                self.seal_array(k, from)
            }
            (Op::And, Runs(x), Runs(y)) => {
                let (mut i, mut j) = (0, 0);
                while i < x.len() && j < y.len() {
                    let (s, l) = (x[i].0.max(y[j].0), x[i].1.min(y[j].1));
                    if s <= l {
                        self.values.extend([s, l]);
                    }
                    if x[i].1 < y[j].1 { i += 1 } else { j += 1 }
                }
                self.seal_runs(k, from)
            }
            (Op::Or, Runs(x), Runs(y)) => {
                let (mut i, mut j) = (0, 0);
                while i < x.len() || j < y.len() {
                    let next = if j == y.len() || (i < x.len() && x[i].0 <= y[j].0) {
                        i += 1;
                        x[i - 1]
                    } else {
                        j += 1;
                        y[j - 1]
                    };
                    let len = self.values.len();
                    if len > from && u32::from(next.0) <= u32::from(self.values[len - 1]) + 1 {
                        self.values[len - 1] = self.values[len - 1].max(next.1);
                    } else {
                        self.values.extend([next.0, next.1]);
                    }
                }
                self.seal_runs(k, from)
            }
            // Bitmaps against bitmaps: whole-slice word operations (they vectorize), the count
            // taken in the same pass.
            (op, Bitmap(x), Bitmap(y)) => {
                let at = self.words.len();
                let n = match op {
                    Op::And => {
                        let len = x.len().min(y.len());
                        self.words.extend_from_slice(&x[..len]);
                        and_count(&mut self.words[at..], &y[..len])
                    }
                    Op::AndNot => {
                        self.words.extend_from_slice(x);
                        let len = x.len().min(y.len());
                        let n = and_not_count(&mut self.words[at..at + len], &y[..len]);
                        n + count(&self.words[at + len..])
                    }
                    Op::Or => {
                        let (long, short) = if x.len() >= y.len() { (x, y) } else { (y, x) };
                        self.words.extend_from_slice(long);
                        or_count(&mut self.words[at..at + short.len()], short) + count(&long[short.len()..])
                    }
                };
                self.seal_counted(k, at, n)
            }
            // A bitmap within runs, without a branch or call per run (runs here average under two
            // words, so those cost more than the words): each run toggles the bit at its start
            // and the bit after its end; a prefix xor over the words turns the toggles into the
            // runs' mask, and-ed with the bitmap and counted in the same pass.
            // Long runs (whole files, `within_files`): the inner words are copied and only the
            // edges masked, then one vectorized count.
            (Op::And, Bitmap(x), Runs(r)) | (Op::And, Runs(r), Bitmap(x))
                if r.iter().map(|&(s, l)| usize::from(l - s) + 1).sum::<usize>() >= 512 * r.len() =>
            {
                let at = self.words.len();
                let len = x.len().min(r.last().map_or(0, |&(_, l)| words_to(l)));
                self.words.resize(at + len, 0);
                let out = &mut self.words[at..];
                for &(s, l) in r {
                    let ws = usize::from(s) / 64;
                    if ws >= len {
                        break;
                    }
                    let wl = usize::from(l) / 64;
                    let lo = !0u64 << (s % 64);
                    let hi = if wl < len { !0u64 >> (63 - l % 64) } else { !0 };
                    let wl = wl.min(len - 1);
                    if ws == wl {
                        out[ws] |= x[ws] & lo & hi;
                        continue;
                    }
                    out[ws] |= x[ws] & lo;
                    out[ws + 1..wl].copy_from_slice(&x[ws + 1..wl]);
                    out[wl] |= x[wl] & hi;
                }
                let n = count(out);
                self.seal_counted(k, at, n)
            }
            (Op::And, Bitmap(x), Runs(r)) | (Op::And, Runs(r), Bitmap(x)) => {
                let at = self.words.len();
                let len = x.len().min(r.last().map_or(0, |&(_, l)| words_to(l)));
                self.words.resize(at + len, 0);
                let out = &mut self.words[at..];
                let end = len * 64;
                for &(s, l) in r {
                    let (s, after) = (usize::from(s), usize::from(l) + 1);
                    if s >= end {
                        break;
                    }
                    out[s / 64] ^= 1 << (s % 64);
                    if after < end {
                        out[after / 64] ^= 1 << (after % 64);
                    }
                }
                let (mut carry, mut n) = (0u64, 0usize);
                for (o, &v) in out.iter_mut().zip(&x[..len]) {
                    let mask = prefix_xor(*o) ^ carry;
                    carry = 0u64.wrapping_sub(mask >> 63);
                    *o = v & mask;
                    n += o.count_ones() as usize;
                }
                self.seal_counted(k, at, n)
            }
            // A bitmap less runs: the runs' mask as for `And`, inverted.
            (Op::AndNot, Bitmap(x), Runs(r)) => {
                let at = self.words.len();
                let len = x.len();
                self.words.resize(at + len, 0);
                let out = &mut self.words[at..];
                let end = len * 64;
                for &(s, l) in r {
                    let (s, after) = (usize::from(s), usize::from(l) + 1);
                    if s >= end {
                        break;
                    }
                    out[s / 64] ^= 1 << (s % 64);
                    if after < end {
                        out[after / 64] ^= 1 << (after % 64);
                    }
                }
                let (mut carry, mut n) = (0u64, 0usize);
                for (o, &v) in out.iter_mut().zip(x) {
                    let mask = prefix_xor(*o) ^ carry;
                    carry = 0u64.wrapping_sub(mask >> 63);
                    *o = v & !mask;
                    n += o.count_ones() as usize;
                }
                self.seal_counted(k, at, n)
            }
            // The rest on words: the result's extent, each side or-ed in (or masked).
            (op, a, b) => {
                let at = self.words.len();
                let extent = match op {
                    Op::And => words_to(a.last().min(b.last())),
                    Op::Or => words_to(a.last().max(b.last())),
                    Op::AndNot => words_to(a.last()),
                };
                self.words.resize(at + extent, 0);
                let out = &mut self.words[at..];
                a.or_into_clipped(out);
                match (op, b) {
                    (Op::Or, b) => b.or_into_clipped(out),
                    (Op::AndNot, Runs(r)) => r.iter().for_each(|&(s, l)| clear_clipped(out, s, l)),
                    (Op::AndNot, Array(y)) => {
                        let limit = out.len() * 64;
                        y.iter().filter(|&&v| usize::from(v) < limit).for_each(|&v| out[usize::from(v) / 64] &= !(1 << (v % 64)));
                    }
                    (op, Bitmap(q)) => {
                        let and = op == Op::And;
                        out.iter_mut().zip(q).for_each(|(o, m)| *o &= if and { *m } else { !*m });
                    }
                    (_, Runs(r)) => {
                        // Clear the gaps between the runs, and after the last.
                        let mut next = 0u32;
                        for &(s, l) in r {
                            if u32::from(s) > next {
                                clear_clipped(out, next as u16, s - 1);
                            }
                            next = u32::from(l) + 1;
                        }
                        if (next as usize) < out.len() * 64 {
                            clear_clipped(out, next as u16, (out.len() * 64 - 1) as u16);
                        }
                    }
                    (_, Array(_)) => unreachable!("an array's intersection is an array"),
                }
                self.seal_words(k, at)
            }
        }
    }

    /// `op` with `other`, chunk by chunk; `keep_left`/`keep_right` copy chunks present on one side.
    fn apply(&self, other: &Self, op: Op) -> Self {
        // The result's buffers reserved once: an intersection's directory is at most the smaller
        // side's, and the others at most both.
        let (d, v, w) = match op {
            // An intersection's bitmaps come from either side's (a bitmap within runs keeps the
            // bitmap's words), so the larger side bounds them.
            Op::And => (self.directory.len().min(other.directory.len()), self.values.len().max(other.values.len()), self.words.len().max(other.words.len())),
            Op::AndNot => (self.directory.len(), self.values.len(), self.words.len()),
            Op::Or => (self.directory.len() + other.directory.len(), self.values.len() + other.values.len(), self.words.len() + other.words.len()),
        };
        let mut out = Self { directory: Vec::with_capacity(d), values: Vec::with_capacity(v), words: Vec::with_capacity(w) };
        let (keep_left, keep_right) = match op {
            Op::And => (false, false),
            Op::Or => (true, true),
            Op::AndNot => (true, false),
        };
        let (a, b) = (&self.directory, &other.directory);
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            let order = match (a.get(i), b.get(j)) {
                (Some(x), Some(y)) => x.key.cmp(&y.key),
                (Some(_), None) => Ordering::Less,
                (None, _) => Ordering::Greater,
            };
            match order {
                Ordering::Less if keep_left => {
                    out.copy_chunk(self, &a[i]);
                    i += 1;
                }
                Ordering::Less => {
                    if j == b.len() {
                        break;
                    }
                    i += a[i..].partition_point(|e| e.key < b[j].key);
                }
                Ordering::Greater if keep_right => {
                    out.copy_chunk(other, &b[j]);
                    j += 1;
                }
                Ordering::Greater => {
                    if i == a.len() {
                        break;
                    }
                    j += b[j..].partition_point(|e| e.key < a[i].key);
                }
                Ordering::Equal => {
                    out.combine(a[i].key, op, self.view(&a[i]), other.view(&b[j]));
                    i += 1;
                    j += 1;
                }
            }
        }
        out
    }

    pub(in crate::code_rule_dsl) fn intersect_with(&mut self, other: &Self) {
        *self = self.apply(other, Op::And);
    }

    /// `self ∩ other` as a new set: an operation writes its result's buffers itself, so its first
    /// operand is not copied first (as `clone` then `intersect_with` would).
    pub(in crate::code_rule_dsl) fn intersection(&self, other: &Self) -> Self {
        self.apply(other, Op::And)
    }

    /// `self − other` as a new set.
    pub(in crate::code_rule_dsl) fn difference(&self, other: &Self) -> Self {
        self.apply(other, Op::AndNot)
    }

    pub(in crate::code_rule_dsl) fn union_with(&mut self, other: &Self) {
        // Appending later files (the common way sets are built) copies the other's buffers once.
        if self.directory.last().is_none_or(|l| other.directory.first().is_none_or(|f| l.key < f.key)) {
            self.append(other, 0);
            return;
        }
        // Only the last chunk shared (later files of the same chunk): that chunk is merged.
        if let (Some(&l), Some(f)) = (self.directory.last(), other.directory.first())
            && l.key == f.key
            && other.directory.get(1).is_none_or(|g| g.key > l.key)
        {
            let mut merged = Self::default();
            merged.combine(l.key, Op::Or, self.view(&l), other.view(f));
            // The last chunk's container is at the end of its buffer: chunks are written in order.
            self.directory.pop();
            if l.form == Form::Bitmap {
                self.words.truncate(l.at as usize);
            } else {
                self.values.truncate(l.at as usize);
            }
            self.append(&merged, 0);
            self.append(other, 1);
            return;
        }
        *self = self.apply(other, Op::Or);
    }

    /// Appends `other`'s chunks from `from` on, which come after every chunk of the set.
    fn append(&mut self, other: &Self, from: usize) {
        let (values, words) = (self.values.len() as u32, self.words.len() as u32);
        if from >= other.directory.len() {
            return;
        }
        let (start_values, start_words) = other.directory[from..].iter().fold((u32::MAX, u32::MAX), |(v, w), e| {
            if e.form == Form::Bitmap { (v, w.min(e.at)) } else { (v.min(e.at), w) }
        });
        let (sv, sw) = (if start_values == u32::MAX { other.values.len() as u32 } else { start_values }, if start_words == u32::MAX { other.words.len() as u32 } else { start_words });
        self.directory.extend(other.directory[from..].iter().map(|e| Entry { at: if e.form == Form::Bitmap { e.at - sw + words } else { e.at - sv + values }, ..*e }));
        self.values.extend_from_slice(&other.values[sv as usize..]);
        self.words.extend_from_slice(&other.words[sw as usize..]);
    }

    #[cfg_attr(not(test), allow(dead_code))] // the programs use `difference`
    pub(in crate::code_rule_dsl) fn difference_with(&mut self, other: &Self) {
        *self = self.apply(other, Op::AndNot);
    }

    /// Keeps the set's nodes in files where each of `tests` has a node.
    pub(in crate::code_rule_dsl) fn within_files(&mut self, space: &NodeSpace, tests: &[&Self]) {
        let Some((first, rest)) = tests.split_first() else { return };
        // The first test's files, kept where every other test has a node too, as ranges: files
        // next to each other are one range.
        let mut kept: Vec<std::ops::Range<u32>> = Vec::new();
        for file in first.files(space) {
            let range = space.range(file);
            if rest.iter().all(|test| test.any_in(range.clone())) {
                match kept.last_mut() {
                    Some(r) if r.end == range.start => r.end = range.end,
                    _ => kept.push(range),
                }
            }
        }
        *self = self.intersection(&Self::from_ranges(&kept));
    }

    /// The set of node ranges `ranges` (sorted, apart): runs containers, written directly.
    fn from_ranges(ranges: &[std::ops::Range<u32>]) -> Self {
        let mut set = Self::default();
        let mut open: Option<(u64, usize)> = None;
        for r in ranges {
            let mut s = r.start;
            while s < r.end {
                let e = r.end.min((s | 0xffff) + 1);
                let (k, a) = key(s);
                if open.is_none_or(|(ok, _)| ok != k) {
                    if let Some((ok, from)) = open.take() {
                        set.seal_runs(ok, from);
                    }
                    open = Some((k, set.values.len()));
                }
                set.values.extend([a, (e - 1) as u16]);
                s = e;
            }
        }
        if let Some((ok, from)) = open {
            set.seal_runs(ok, from);
        }
        set
    }

    /// The files holding a member, ascending, each once. A sparse chunk's members are walked with
    /// a cursor over the files; a dense one asks once per file it overlaps.
    fn files(&self, space: &NodeSpace) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let files = space.files();
        let mut file = 0usize;
        for e in &self.directory {
            let base = (e.key as u32) << 16;
            match self.view(e) {
                View::Array(a) => {
                    for &v in a {
                        let node = base | u32::from(v);
                        while space.range(file).end <= node {
                            file += 1;
                        }
                        if out.last() != Some(&file) {
                            out.push(file);
                        }
                    }
                }
                view => {
                    while file < files && space.range(file).end <= base {
                        file += 1;
                    }
                    while file < files && space.base(file) < base + 65_536 {
                        let range = space.range(file);
                        if !range.is_empty() {
                            let (a, b) = (range.start.max(base) - base, (range.end - 1).min(base + 65_535) - base);
                            if out.last() != Some(&file) && view.any_between(a as u16, b as u16) {
                                out.push(file);
                            }
                        }
                        if range.end > base + 65_536 {
                            break;
                        }
                        file += 1;
                    }
                }
            }
        }
        out
    }

    /// Whether a member is in `range` (of nodes).
    fn any_in(&self, range: std::ops::Range<u32>) -> bool {
        if range.is_empty() {
            return false;
        }
        let (first, last) = (range.start >> 16, (range.end - 1) >> 16);
        let mut at = self.directory.partition_point(|e| e.key < u64::from(first));
        while let Some(e) = self.directory.get(at) {
            let k = e.key as u32;
            if k > last {
                return false;
            }
            let a = if k == first { range.start as u16 } else { 0 };
            let b = if k == last { (range.end - 1) as u16 } else { u16::MAX };
            if self.view(e).any_between(a, b) {
                return true;
            }
            at += 1;
        }
        false
    }

    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.directory.iter().map(|e| self.view(e).len()).sum()
    }

    // The contract's membership test; no caller yet.
    #[allow(dead_code)]
    pub(in crate::code_rule_dsl) fn contains(&self, node: u32) -> bool {
        let (k, v) = key(node);
        let at = self.directory.partition_point(|e| e.key < k);
        self.directory.get(at).is_some_and(|e| e.key == k && self.view(e).contains(v))
    }

    /// The members (node numbers), ascending.
    pub(in crate::code_rule_dsl) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.directory.iter().flat_map(|e| self.chunk_nodes(e))
    }

    fn chunk_nodes(&self, e: &Entry) -> impl Iterator<Item = u32> + '_ {
        let high = (e.key as u32) << 16;
        let mut values = Vec::new();
        self.view(e).for_each(|v| values.push(v));
        values.into_iter().map(move |v| high | u32::from(v))
    }

    /// The set's nodes of file `file`, as positions in the file, ascending.
    pub fn file_nodes<'a>(&'a self, space: &'a NodeSpace, file: usize) -> impl Iterator<Item = u32> + 'a {
        let range = space.range(file);
        let (first, last) = (u64::from(range.start >> 16), u64::from(range.end.saturating_sub(1) >> 16));
        let from = self.directory.partition_point(|e| e.key < first);
        let to = if range.is_empty() { from } else { from + self.directory[from..].partition_point(|e| e.key <= last) };
        let (start, end) = (range.start, range.end);
        self.directory[from..to].iter().flat_map(move |e| self.chunk_nodes(e).filter(move |&n| n >= start && n < end).map(move |n| n - start))
    }

    /// Bytes held: the directory and the buffers.
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.directory.len() * std::mem::size_of::<Entry>() + 2 * self.values.len() + 8 * self.words.len()
    }

    /// File `file`'s members as a stored segment (0045 §5): the file's own 2^16 slices, per slice
    /// its number (u16), its form (u8), its length (u32) and its container, little-endian.
    #[cfg_attr(not(test), allow(dead_code))] // tests and measurements
    pub(in crate::code_rule_dsl) fn segment(&self, space: &NodeSpace, file: usize) -> Vec<u8> {
        let mut b = Builder::default();
        self.file_nodes(space, file).for_each(|p| b.node(p));
        b.finish().encode()
    }

    /// The set's chunks as a segment: for a set of one file's positions (a space of one file).
    pub(in crate::code_rule_dsl) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.directory {
            out.extend_from_slice(&(e.key as u16).to_le_bytes());
            out.push(e.form as u8);
            out.extend_from_slice(&e.len.to_le_bytes());
            let (at, len) = (e.at as usize, e.len as usize);
            match e.form {
                Form::Array => self.values[at..at + len].iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes())),
                Form::Runs => self.values[at..at + 2 * len].iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes())),
                Form::Bitmap => self.words[at..at + len].iter().for_each(|w| out.extend_from_slice(&w.to_le_bytes())),
            }
        }
        out
    }

    /// The same members with every chunk a bitmap (to measure what runs cost against words).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn as_bitmaps(&self) -> Self {
        let mut out = Self::default();
        for e in &self.directory {
            let v = self.view(e);
            let mut b = vec![0u64; words_to(v.last())];
            v.or_into(&mut b);
            out.directory.push(Entry { key: e.key, form: Form::Bitmap, at: out.words.len() as u32, len: b.len() as u32 });
            out.words.extend_from_slice(&b);
        }
        out
    }

    /// (array, bitmap, runs) chunk counts.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn shape(&self) -> (usize, usize, usize) {
        let mut s = (0, 0, 0);
        for e in &self.directory {
            match e.form {
                Form::Array => s.0 += 1,
                Form::Bitmap => s.1 += 1,
                Form::Runs => s.2 += 1,
            }
        }
        s
    }

    /// Per form: (chunks, containers' elements: values, words or runs).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn census(&self) -> [(usize, usize); 3] {
        let mut s = [(0, 0); 3];
        for e in &self.directory {
            let at = match e.form {
                Form::Array => 0,
                Form::Bitmap => 1,
                Form::Runs => 2,
            };
            s[at].0 += 1;
            s[at].1 += e.len as usize;
        }
        s
    }
}

/// Builds a set in node order: each chunk's members gather in a bitmap of the chunk, and the chunk
/// is sealed in the form that suits it (`seal_array`'s rule) when a later chunk starts.
pub(in crate::code_rule_dsl) struct Builder {
    set: NodeSet,
    key: Option<u64>,
    /// The open chunk's members while it has only single nodes, few enough to seal as an array
    /// without scanning the chunk's words.
    pending: Vec<u16>,
    /// Buffers reused to read stored containers.
    u16s: Vec<u16>,
    u64s: Vec<u64>,
    /// Whether the open chunk is in `scratch`.
    dense: bool,
    scratch: Vec<u64>,
    /// The words written in the open chunk.
    lo: usize,
    hi: usize,
}

impl Default for Builder {
    fn default() -> Self {
        Self { set: NodeSet::default(), key: None, pending: Vec::new(), u16s: Vec::new(), u64s: Vec::new(), dense: false, scratch: vec![0; 1024], lo: usize::MAX, hi: 0 }
    }
}

impl Builder {
    fn open(&mut self, k: u64) {
        if self.key != Some(k) {
            assert!(self.key.is_none_or(|open| open < k), "members are added in chunk order");
            self.seal();
            self.key = Some(k);
        }
    }

    /// Moves the open chunk's pending nodes to `scratch`.
    fn spill(&mut self) {
        if self.dense {
            return;
        }
        self.dense = true;
        for i in 0..self.pending.len() {
            let v = self.pending[i];
            let w = usize::from(v) / 64;
            self.scratch[w] |= 1 << (v % 64);
            self.touch(w, w);
        }
        self.pending.clear();
    }

    fn touch(&mut self, from: usize, to: usize) {
        self.lo = self.lo.min(from);
        self.hi = self.hi.max(to);
    }

    /// Adds node `node`: in a chunk at or after every chunk added to (any order within one).
    pub(in crate::code_rule_dsl) fn node(&mut self, node: u32) {
        let (k, v) = key(node);
        self.open(k);
        if !self.dense {
            match self.pending.last() {
                Some(&last) if last == v => return,
                // Out of order within the chunk: the chunk's words take any order.
                Some(&last) if last > v => self.spill(),
                _ if self.pending.len() < 1024 => {
                    self.pending.push(v);
                    return;
                }
                _ => self.spill(),
            }
        }
        let w = usize::from(v) / 64;
        self.scratch[w] |= 1 << (v % 64);
        self.touch(w, w);
    }

    /// Adds nodes `start..end`.
    pub(in crate::code_rule_dsl) fn run(&mut self, start: u32, end: u32) {
        let mut s = start;
        while s < end {
            let e = end.min((s | 0xffff) + 1);
            let (k, a) = key(s);
            self.open(k);
            self.spill();
            let b = (e - 1) as u16;
            fill(&mut self.scratch, a, b);
            self.touch(usize::from(a) / 64, usize::from(b) / 64);
            s = e;
        }
    }

    /// Adds the members of `words`, whose bit `j` of word `i` is node `start + 64 i + j`: per
    /// chunk the words it covers, shifted into place in one pass.
    pub(in crate::code_rule_dsl) fn words_at(&mut self, start: u32, words: &[u64]) {
        let mut i = 0;
        let mut carry = 0u64;
        while i < words.len() {
            let (k, v) = key(start + 64 * i as u32);
            let (at, shift) = (usize::from(v) / 64, u32::from(v % 64));
            let m = (words.len() - i).min(1024 - at);
            let part = &words[i..i + m];
            self.open(k);
            self.spill();
            let out = &mut self.scratch[at..at + m];
            if shift == 0 {
                out.iter_mut().zip(part).for_each(|(o, &w)| *o |= w);
            } else {
                out[0] |= part[0] << shift | carry;
                for j in 1..m {
                    out[j] |= part[j] << shift | part[j - 1] >> (64 - shift);
                }
                carry = part[m - 1] >> (64 - shift);
            }
            let mut last = at + m - 1;
            if carry != 0 && at + m < 1024 {
                self.scratch[at + m] |= carry;
                carry = 0;
                last += 1;
            }
            self.touch(at, last);
            i += m;
        }
        if carry != 0 {
            // The last word's high bits, at the start of the next chunk.
            let (k, _) = key(start + 64 * words.len() as u32);
            self.open(k);
            self.spill();
            self.scratch[0] |= carry;
            self.touch(0, 0);
        }
    }

    /// Adds nodes `offset + v` for `values`, ascending: per chunk one slice, appended to the
    /// pending array when it fits, else set in the chunk's words.
    fn array_at(&mut self, offset: u32, values: &[u16]) {
        let mut rest = values;
        while let Some(&first) = rest.first() {
            let (k, _) = key(offset + u32::from(first));
            let end = (k + 1) << 16;
            let split = rest.partition_point(|&v| u64::from(offset + u32::from(v)) < end);
            let (part, tail) = rest.split_at(split);
            self.open(k);
            let add = offset as u16;
            let lowest = first.wrapping_add(add);
            if !self.dense && self.pending.last().is_none_or(|&l| l < lowest) && self.pending.len() + part.len() <= 1024 {
                self.pending.extend(part.iter().map(|&v| v.wrapping_add(add)));
            } else {
                self.spill();
                for &v in part {
                    let x = v.wrapping_add(add);
                    self.scratch[usize::from(x) / 64] |= 1 << (x % 64);
                }
                let highest = part[part.len() - 1].wrapping_add(add);
                self.touch(usize::from(lowest) / 64, usize::from(highest) / 64);
            }
            rest = tail;
        }
    }

    /// Adds a stored segment (`NodeSet::encode`) of the file at `base`, whose positions are below
    /// `count`: arrays and runs move by `base`, bitmaps are shifted into the space's chunks. The
    /// containers are read in bulk into reused buffers.
    pub(in crate::code_rule_dsl) fn segment(&mut self, base: u32, mut bytes: &[u8], count: u32) -> Result<(), &'static str> {
        let invalid = "invalid segment";
        let past = "a position past the file's nodes";
        let mut last: Option<u16> = None;
        let (mut u16s, mut u64s) = (std::mem::take(&mut self.u16s), std::mem::take(&mut self.u64s));
        let result = (|| {
            while !bytes.is_empty() {
                let (head, rest) = bytes.split_at_checked(7).ok_or(invalid)?;
                let slice = u16::from_le_bytes([head[0], head[1]]);
                let len = u32::from_le_bytes([head[3], head[4], head[5], head[6]]) as usize;
                if last.is_some_and(|l| l >= slice) || len == 0 {
                    return Err(invalid);
                }
                last = Some(slice);
                let size = match head[2] {
                    0 => 2 * len,
                    1 => 8 * len,
                    2 => 4 * len,
                    _ => return Err(invalid),
                };
                let (payload, rest) = rest.split_at_checked(size).ok_or(invalid)?;
                let from = u32::from(slice) << 16;
                match head[2] {
                    0 => {
                        le_u16s(payload, &mut u16s);
                        if !u16s.windows(2).all(|p| p[0] < p[1]) {
                            return Err(invalid);
                        }
                        if from | u32::from(u16s[len - 1]) >= count {
                            return Err(past);
                        }
                        self.array_at(base + from, &u16s);
                    }
                    1 => {
                        le_u64s(payload, &mut u64s);
                        if u64s[len - 1] == 0 || from + View::Bitmap(&u64s).last() as u32 >= count {
                            return Err(past);
                        }
                        self.words_at(base + from, &u64s);
                    }
                    _ => {
                        le_u16s(payload, &mut u16s);
                        for pair in u16s.chunks_exact(2) {
                            let (s, l) = (pair[0], pair[1]);
                            let top = from | u32::from(l);
                            if s > l || top >= count {
                                return Err(past);
                            }
                            self.run(base + (from | u32::from(s)), base + top + 1);
                        }
                    }
                }
                bytes = rest;
            }
            Ok(())
        })();
        (self.u16s, self.u64s) = (u16s, u64s);
        result
    }

    /// Seals the open chunk.
    fn seal(&mut self) {
        let Some(k) = self.key.take() else { return };
        if !self.dense {
            let from = self.set.values.len();
            self.set.values.extend_from_slice(&self.pending);
            self.pending.clear();
            self.set.seal_array(k, from);
            return;
        }
        self.dense = false;
        // Words touched may be zero (a bitmap's words are or-ed in whole): trim to the members.
        while self.lo <= self.hi && self.scratch[self.hi] == 0 {
            if self.hi == 0 {
                break;
            }
            self.hi -= 1;
        }
        if self.lo > self.hi || self.scratch[self.hi] == 0 {
            if self.lo <= self.hi {
                self.scratch[self.lo..=self.hi].fill(0);
            }
            self.lo = usize::MAX;
            self.hi = 0;
            return;
        }
        let (lo, hi) = (self.lo, self.hi);
        let words = &self.scratch[..=hi];
        let n = count(&words[lo..]);
        // Runs: the 0 -> 1 transitions.
        let mut runs = 0usize;
        let mut prev = 0u64;
        for &w in &words[lo..] {
            runs += (w & !((w << 1) | (prev >> 63))).count_ones() as usize;
            prev = w;
        }
        let w = hi + 1;
        let set = &mut self.set;
        let from = set.values.len();
        if 4 * runs < (2 * n).min(8 * w) {
            // Two states per bit scan: outside a run the next set bit starts one; inside, the
            // next clear bit ends it.
            let mut start: Option<u32> = None;
            for (at, &word) in words.iter().enumerate().skip(lo) {
                let base = (at * 64) as u32;
                let mut x = word;
                loop {
                    match start {
                        None => {
                            if x == 0 {
                                break;
                            }
                            let t = x.trailing_zeros();
                            start = Some(base + t);
                            x |= (1u64 << t) - 1;
                        }
                        Some(s) => {
                            let clear = !x;
                            if clear == 0 {
                                break;
                            }
                            let t = clear.trailing_zeros();
                            set.values.extend([s as u16, (base + t - 1) as u16]);
                            start = None;
                            x &= !((1u64 << t) - 1);
                        }
                    }
                }
            }
            if let Some(s) = start {
                set.values.extend([s as u16, (hi * 64 + 63) as u16]);
            }
            let len = (set.values.len() - from) / 2;
            debug_assert_eq!(len, runs);
            set.directory.push(Entry { key: k, form: Form::Runs, at: from as u32, len: len as u32 });
        } else if n > w {
            set.directory.push(Entry { key: k, form: Form::Bitmap, at: set.words.len() as u32, len: w as u32 });
            set.words.extend_from_slice(words);
        } else {
            push_bits(&words[lo..], lo * 64, n, &mut set.values);
            set.directory.push(Entry { key: k, form: Form::Array, at: from as u32, len: n as u32 });
        }
        self.scratch[lo..=hi].fill(0);
        self.lo = usize::MAX;
        self.hi = 0;
    }

    pub(in crate::code_rule_dsl) fn finish(mut self) -> NodeSet {
        self.seal();
        self.set
    }
}

impl View<'_> {
    /// `or_into` for an `out` that may be shorter than the members (they are cut at its end).
    fn or_into_clipped(self, out: &mut [u64]) {
        let limit = out.len() * 64;
        match self {
            View::Array(a) => a.iter().take_while(|&&v| usize::from(v) < limit).for_each(|&v| out[usize::from(v) / 64] |= 1 << (v % 64)),
            View::Runs(r) => {
                for &(s, l) in r {
                    if usize::from(s) >= limit {
                        break;
                    }
                    fill(out, s, l.min((limit - 1) as u16));
                }
            }
            View::Bitmap(b) => out.iter_mut().zip(b).for_each(|(o, w)| *o |= w),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A small deterministic generator (no dependency).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }

    const SPANS: [u32; 4] = [100, 70_000, 140_000, 5_000];

    fn space() -> NodeSpace {
        NodeSpace::new(SPANS.to_vec())
    }

    /// Sets of every shape: sparse, dense, runs, across chunk edges and files (whose bases are not
    /// aligned to words or chunks).
    fn sample(g: &mut Lcg, shape: u32) -> (NodeSet, BTreeSet<u32>) {
        let space = space();
        let mut b = Builder::default();
        let mut model = BTreeSet::new();
        for file in 0..4usize {
            let (span, base) = (SPANS[file], space.base(file));
            if shape % 3 == 2 {
                let mut at = g.next() % 50;
                while at < span {
                    let len = 1 + g.next() % 20_000;
                    let end = (at + len).min(span);
                    (at..end).for_each(|p| {
                        model.insert(base + p);
                    });
                    b.run(base + at, base + end);
                    at = end + 1 + g.next() % 30_000;
                }
            } else {
                let density = if shape % 3 == 0 { 50 } else { 2 };
                for p in (0..span).filter(|_| g.next() % density == 0) {
                    model.insert(base + p);
                    b.node(base + p);
                }
            }
        }
        (b.finish(), model)
    }

    #[test]
    fn chunked_sets_agree_with_ordered_sets() {
        let mut g = Lcg(7);
        let space = space();
        for round in 0..12u32 {
            let (a, ma) = sample(&mut g, round);
            let (b, mb) = sample(&mut g, round / 3 + round);
            assert_eq!(a.len(), ma.len());
            assert!(a.iter().eq(ma.iter().copied()));
            let i: BTreeSet<_> = ma.intersection(&mb).copied().collect();
            let u: BTreeSet<_> = ma.union(&mb).copied().collect();
            let d: BTreeSet<_> = ma.difference(&mb).copied().collect();
            let (mut i2, mut u2, mut d2) = (a.clone(), a.clone(), a.clone());
            i2.intersect_with(&b);
            u2.union_with(&b);
            d2.difference_with(&b);
            assert!(i2.iter().eq(i.iter().copied()), "intersect, round {round}");
            assert!(u2.iter().eq(u.iter().copied()), "union, round {round}");
            assert!(d2.iter().eq(d.iter().copied()), "difference, round {round}");
            // Dense words round-trip (the Metal bridge).
            assert_eq!(NodeSet::from_words(&space, &a.to_words(&space)), a, "words, round {round}");
            for &n in ma.iter().take(50) {
                assert!(a.contains(n));
            }
            // Per file: positions, and within files.
            for file in 0..4 {
                let range = space.range(file);
                let want: Vec<u32> = ma.range(range.clone()).map(|n| n - range.start).collect();
                assert!(a.file_nodes(&space, file).eq(want.iter().copied()), "file_nodes, round {round} file {file}");
            }
            let mut w = a.clone();
            w.within_files(&space, &[&b]);
            let held = |f: usize| mb.range(space.range(f)).next().is_some();
            let want: Vec<u32> = ma.iter().copied().filter(|&n| held((0..4).find(|&f| space.range(f).contains(&n)).unwrap())).collect();
            assert!(w.iter().eq(want.iter().copied()), "within, round {round}");
        }
    }

    #[test]
    fn files_of_a_chunk_append_in_order() {
        // Files of one chunk inserted one by one: the shared chunk is merged, not rebuilt.
        let space = NodeSpace::new(vec![10, 10, 10]);
        let mut s = NodeSet::empty(&space);
        s.insert_file(&space, 0, [1, 2]);
        s.insert_file(&space, 1, [0]);
        s.insert_runs(&space, 2, [3..7]);
        assert!(s.iter().eq([1, 2, 10, 23, 24, 25, 26]));
        assert_eq!(s.directory.len(), 1);
    }

    #[test]
    fn containers_follow_the_members() {
        let mut b = Builder::default();
        [1, 500, 9_000].iter().for_each(|&n| b.node(n));
        b.run(65_536 + 10, 65_536 + 60_000);
        (2 * 65_536..2 * 65_536 + 10_000).for_each(|n| b.node(n));
        [1, 5, 9].iter().for_each(|&n| b.node(3 * 65_536 + n));
        assert_eq!(b.finish().shape(), (1, 1, 2), "sparse: array; a subtree: runs; a dense block: runs; more members than words: bitmap");
        let mut d = Builder::default();
        (0..65_536).step_by(2).for_each(|n| d.node(n));
        assert_eq!(d.finish().shape(), (0, 1, 0), "half the chunk, no runs: bitmap");
    }

    /// A file's segment is the store's format: `segment_positions` reads the same positions, and
    /// `Builder::segment` puts them back at the file's base, for every form.
    #[test]
    fn segments_are_the_stores_format() {
        let mut g = Lcg(11);
        let space = space();
        for round in 0..6u32 {
            let (set, model) = sample(&mut g, round);
            let mut back = Builder::default();
            for file in 0..4usize {
                let bytes = set.segment(&space, file);
                let range = space.range(file);
                let want: Vec<u32> = model.range(range.clone()).map(|n| n - range.start).collect();
                assert_eq!(crate::content_store::segment_positions(&bytes).unwrap(), want, "round {round} file {file}");
                back.segment(range.start, &bytes, space.count(file)).unwrap();
                if let Some(&last) = want.last() {
                    assert!(Builder::default().segment(range.start, &bytes, last).is_err(), "a position past the file is refused");
                }
            }
            assert_eq!(back.finish(), set, "round {round}");
        }
    }
}
