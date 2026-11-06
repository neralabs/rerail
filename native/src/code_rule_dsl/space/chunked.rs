//! Node sets as chunked containers (decision 0045).
//!
//! A node is (file, position in the file). Positions are cut into chunks of 2^16; each chunk that
//! holds a member is one container, found in a sorted directory by `file << 16 | position >> 16`:
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
//! smaller. A bitmap is priced at the chunk's extent, not at 2^16 bits as in Roaring, because a
//! document is mostly much smaller than a chunk.

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

/// A set of a space's nodes: per file of the space, its members as chunked containers. File
/// numbers are the space's (`NodeSpace::base`'s order).
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

/// A node's directory key (file, its 2^16 slice) and its value in the chunk.
fn key(file: u32, position: u32) -> (u64, u16) {
    ((u64::from(file) << 16) | u64::from(position >> 16), position as u16)
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

    /// Every node of the space: one run per file chunk.
    pub(in crate::code_rule_dsl) fn all(space: &NodeSpace) -> Self {
        let mut set = Self::default();
        (0..space.files()).for_each(|file| set.push_file_ranges(file as u32, [0..space.count(file)]));
        set
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
    pub(in crate::code_rule_dsl) fn insert_file(&mut self, file: usize, positions: impl IntoIterator<Item = u32>) {
        let mut add = Self::default();
        add.push_file(file as u32, positions);
        self.union_with(&add);
    }

    /// File `file`'s runs `ranges` (sorted, apart), added to the set: §3's tree axes build sets this way.
    #[allow(dead_code)] // no tree-axis set is built on the host yet
    pub(in crate::code_rule_dsl) fn insert_runs(&mut self, file: usize, ranges: impl IntoIterator<Item = std::ops::Range<u32>>) {
        let mut add = Self::default();
        add.push_file_ranges(file as u32, ranges);
        self.union_with(&add);
    }

    /// Adds every node of file `file`.
    pub(in crate::code_rule_dsl) fn fill_file(&mut self, space: &NodeSpace, file: usize) {
        let mut add = Self::default();
        add.push_file_ranges(file as u32, [0..space.count(file)]);
        self.union_with(&add);
    }

    /// The set of `words`, a slot of the space's dense words (Metal's `set_steps`).
    pub(in crate::code_rule_dsl) fn from_words(space: &NodeSpace, words: &[u64]) -> Self {
        let mut set = Self::default();
        for file in 0..space.files() {
            let range = space.range(file);
            let base = range.start;
            let positions = range.filter(|&node| words[node as usize / 64] >> (node % 64) & 1 != 0).map(|node| node - base);
            set.push_file(file as u32, positions);
        }
        set
    }

    /// The set as the space's dense words (Metal's `set_steps`).
    pub(in crate::code_rule_dsl) fn to_words(&self, space: &NodeSpace) -> Vec<u64> {
        let mut words = vec![0_u64; space.words()];
        for (file, position) in self.iter() {
            let node = space.base(file as usize) + position;
            words[node as usize / 64] |= 1 << (node % 64);
        }
        words
    }

    /// Adds the positions (ascending) of file `file`, after every file already in the set.
    fn push_file(&mut self, file: u32, positions: impl IntoIterator<Item = u32>) {
        let mut current: Option<u64> = None;
        let start = self.values.len();
        let mut from = start;
        for p in positions {
            let (k, v) = key(file, p);
            if current != Some(k) {
                if let Some(ck) = current {
                    self.seal_array(ck, from);
                }
                current = Some(k);
                from = self.values.len();
            }
            self.values.push(v);
        }
        if let Some(ck) = current {
            self.seal_array(ck, from);
        }
    }

    /// Adds the runs `ranges` (sorted, apart) of file `file`, after every file already in the set.
    fn push_file_ranges(&mut self, file: u32, ranges: impl IntoIterator<Item = std::ops::Range<u32>>) {
        let mut current: Option<u64> = None;
        let mut from = self.values.len();
        for r in ranges {
            let mut s = r.start;
            while s < r.end {
                let e = r.end.min((s | 0xffff) + 1);
                let (k, lo) = key(file, s);
                if current != Some(k) {
                    if let Some(ck) = current {
                        self.seal_runs(ck, from);
                    }
                    current = Some(k);
                    from = self.values.len();
                }
                self.values.extend([lo, (e - 1) as u16]);
                s = e;
            }
        }
        if let Some(ck) = current {
            self.seal_runs(ck, from);
        }
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
            self.values.extend(bits(&b));
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
            self.values.extend(bits(&self.words[from..]));
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
            self.values.extend(bits(&self.words[from..]));
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
            (Op::AndNot, Bitmap(x), Runs(r)) => {
                let at = self.words.len();
                self.words.extend_from_slice(x);
                let out = &mut self.words[at..];
                r.iter().for_each(|&(s, l)| clear_clipped(out, s, l));
                self.seal_words(k, at)
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
        // The result's buffers reserved once: an intersection is at most the smaller side, and the
        // others at most both.
        let (d, v, w) = match op {
            Op::And => (self.directory.len().min(other.directory.len()), self.values.len().min(other.values.len()), self.words.len().min(other.words.len())),
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
            let (values, words) = (self.values.len() as u32, self.words.len() as u32);
            self.directory.extend(other.directory.iter().map(|e| Entry { at: e.at + if e.form == Form::Bitmap { words } else { values }, ..*e }));
            self.values.extend_from_slice(&other.values);
            self.words.extend_from_slice(&other.words);
            return;
        }
        *self = self.apply(other, Op::Or);
    }

    #[cfg_attr(not(test), allow(dead_code))] // the programs use `difference`
    pub(in crate::code_rule_dsl) fn difference_with(&mut self, other: &Self) {
        *self = self.apply(other, Op::AndNot);
    }

    /// Keeps the set's nodes in files where each of `tests` has a node.
    pub(in crate::code_rule_dsl) fn within_files(&mut self, _space: &NodeSpace, tests: &[&Self]) {
        let mut out = Self::default();
        let mut at = vec![0_usize; tests.len()];
        for e in &self.directory {
            let file = e.key >> 16;
            let held = tests.iter().zip(at.iter_mut()).all(|(test, j)| {
                *j += test.directory[*j..].partition_point(|t| t.key >> 16 < file);
                test.directory.get(*j).is_some_and(|t| t.key >> 16 == file)
            });
            if held {
                out.copy_chunk(self, e);
            }
        }
        *self = out;
    }

    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.directory.iter().map(|e| self.view(e).len()).sum()
    }

    // The contract's membership test; no caller yet.
    #[allow(dead_code)]
    pub(in crate::code_rule_dsl) fn contains(&self, file: usize, position: u32) -> bool {
        let (k, v) = key(file as u32, position);
        let at = self.directory.partition_point(|e| e.key < k);
        self.directory.get(at).is_some_and(|e| e.key == k && self.view(e).contains(v))
    }

    /// The members as (file, position), ascending.
    pub(in crate::code_rule_dsl) fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.directory.iter().flat_map(|e| self.chunk_nodes(e))
    }

    fn chunk_nodes(&self, e: &Entry) -> impl Iterator<Item = (u32, u32)> + '_ {
        let (file, high) = ((e.key >> 16) as u32, ((e.key & 0xffff) as u32) << 16);
        let mut values = Vec::new();
        self.view(e).for_each(|v| values.push(v));
        values.into_iter().map(move |v| (file, high | u32::from(v)))
    }

    /// The set's nodes of file `file`, as positions in the file, ascending.
    pub(in crate::code_rule_dsl) fn file_nodes<'a>(&'a self, _space: &'a NodeSpace, file: usize) -> impl Iterator<Item = u32> + 'a {
        let file = file as u64;
        let from = self.directory.partition_point(|e| e.key >> 16 < file);
        let to = from + self.directory[from..].partition_point(|e| e.key >> 16 == file);
        self.directory[from..to].iter().flat_map(|e| self.chunk_nodes(e).map(|(_, p)| p))
    }

    /// Bytes held: the directory and the buffers.
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.directory.len() * std::mem::size_of::<Entry>() + 2 * self.values.len() + 8 * self.words.len()
    }

    /// File `file`'s chunks as a stored segment (0045 §5): per chunk its slice (u16), its form
    /// (u8), its length (u32) and its container, little-endian. Read back by `insert_segment`
    /// without decoding member by member.
    pub(in crate::code_rule_dsl) fn segment(&self, file: usize) -> Vec<u8> {
        let file = file as u64;
        let from = self.directory.partition_point(|e| e.key >> 16 < file);
        let mut out = Vec::new();
        for e in self.directory[from..].iter().take_while(|e| e.key >> 16 == file) {
            out.extend_from_slice(&((e.key & 0xffff) as u16).to_le_bytes());
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

    /// Adds a segment of file `file` (`segment`), whose positions are below `count`, after every
    /// file already in the set.
    pub(in crate::code_rule_dsl) fn push_segment(&mut self, file: usize, mut bytes: &[u8], count: u32) -> Result<(), &'static str> {
        let invalid = "invalid segment";
        while !bytes.is_empty() {
            let (head, rest) = bytes.split_at_checked(7).ok_or(invalid)?;
            let slice = u16::from_le_bytes([head[0], head[1]]);
            let len = u32::from_le_bytes([head[3], head[4], head[5], head[6]]) as usize;
            let k = (file as u64) << 16 | u64::from(slice);
            if self.directory.last().is_some_and(|e| e.key >= k) || len == 0 {
                return Err(invalid);
            }
            let (form, size) = match head[2] {
                0 => (Form::Array, 2 * len),
                1 => (Form::Bitmap, 8 * len),
                2 => (Form::Runs, 4 * len),
                _ => return Err(invalid),
            };
            let (payload, rest) = rest.split_at_checked(size).ok_or(invalid)?;
            let at = if form == Form::Bitmap {
                let at = self.words.len();
                self.words.extend(payload.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes"))));
                at
            } else {
                let at = self.values.len();
                self.values.extend(payload.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])));
                at
            };
            self.directory.push(Entry { key: k, form, at: at as u32, len: len as u32 });
            let e = *self.directory.last().expect("just pushed");
            if (u32::from(slice) << 16 | u32::from(self.view(&e).last())) >= count {
                return Err("a position past the file's nodes");
            }
            bytes = rest;
        }
        Ok(())
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

    /// Sets of every shape: sparse, dense, runs, across chunk edges and files.
    fn sample(g: &mut Lcg, shape: u32) -> (NodeSet, BTreeSet<(u32, u32)>) {
        let mut set = NodeSet::default();
        let mut model = BTreeSet::new();
        for file in 0..4u32 {
            let span = [100u32, 70_000, 140_000, 5_000][file as usize];
            if shape % 3 == 2 {
                let mut ranges = Vec::new();
                let mut at = g.next() % 50;
                while at < span {
                    let len = 1 + g.next() % 20_000;
                    let end = (at + len).min(span);
                    ranges.push(at..end);
                    at = end + 1 + g.next() % 30_000;
                }
                ranges.iter().for_each(|r| r.clone().for_each(|p| { model.insert((file, p)); }));
                set.push_file_ranges(file, ranges);
            } else {
                let density = if shape % 3 == 0 { 50 } else { 2 };
                let ps: Vec<u32> = (0..span).filter(|_| g.next() % density == 0).collect();
                ps.iter().for_each(|&p| { model.insert((file, p)); });
                set.push_file(file, ps);
            }
        }
        (set, model)
    }

    #[test]
    fn chunked_sets_agree_with_ordered_sets() {
        let mut g = Lcg(7);
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
            let space = NodeSpace::new(vec![100, 70_000, 140_000, 5_000]);
            assert_eq!(NodeSet::from_words(&space, &a.to_words(&space)), a, "words, round {round}");
            for &(f, p) in ma.iter().take(50) {
                assert!(a.contains(f as usize, p));
            }
        }
    }

    #[test]
    fn containers_follow_the_members() {
        let mut s = NodeSet::default();
        s.push_file(0, [1, 500, 9_000]);
        s.push_file_ranges(1, [10..60_000]);
        s.push_file(2, 0..10_000);
        s.push_file(3, [1, 5, 9]);
        assert_eq!(s.shape(), (1, 1, 2), "sparse: array; a subtree: runs; a dense block: runs; more members than words: bitmap");
        let mut d = NodeSet::default();
        d.push_file(0, (0..65_536).step_by(2));
        assert_eq!(d.shape(), (0, 1, 0), "half the chunk, no runs: bitmap");
    }

    /// A segment written here is the store's format: `segment_positions` reads the same members,
    /// and `push_segment` the same set, for every form.
    #[test]
    fn segments_are_the_stores_format() {
        let mut g = Lcg(11);
        for round in 0..6u32 {
            let (set, model) = sample(&mut g, round);
            for file in 0..4u32 {
                let bytes = set.segment(file as usize);
                let want: Vec<u32> = model.iter().filter(|(f, _)| *f == file).map(|&(_, p)| p).collect();
                assert_eq!(crate::content_store::segment_positions(&bytes).unwrap(), want, "round {round} file {file}");
                let mut back = NodeSet::default();
                back.push_segment(file as usize, &bytes, 200_000).unwrap();
                assert!(back.iter().map(|(_, p)| p).eq(want.iter().copied()));
                if let Some(&last) = want.last() {
                    assert!(NodeSet::default().push_segment(file as usize, &bytes, last).is_err(), "a position past the file is refused");
                }
            }
        }
    }
}
