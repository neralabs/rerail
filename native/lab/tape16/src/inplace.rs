//! E10 (0156 T9, t38): in-memory tapes read in place. Each candidate is built from a file's tree, and
//! every question is answered on its own bytes, with nothing built beside it.
//!
//! - `F15`: format 15's image as it is (code, size, base + off, tail; the code table per tape).
//! - `X`: explicit, bit-packed a block at a time: code, subtree size, back distance to the parent,
//!   start (frame of reference) and length, each with patched exceptions (PFOR).
//! - `B`: balanced parentheses (format 16's events, one bit an event) with rank samples and excess
//!   summaries, and the same code, start and length arrays as `X`, indexed by preorder rank. A node is
//!   the position of its open, as simdjson's tape names a value by its index.
//!
//! The questions: `code` (kind, field, flags, classes through the language's table), `subtree_end`, `parent`,
//! `start`, `end`; children by first child and next sibling.

use crate::tree::{NONE, Tree};

pub const BLOCK: usize = 256;

/// A candidate's in-memory bytes, as they would be written to disk (and zstd'd there): every array, raw.
pub trait Image {
    fn image(&self, out: &mut Vec<u8>);
}

pub(crate) fn raw<T: Copy>(out: &mut Vec<u8>, v: &[T]) {
    // SAFETY: plain integers, read as bytes.
    out.extend_from_slice(unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) });
}

#[inline(always)]
fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

/// `w` bits at bit `bit` of `words` (one padding word at the end).
#[inline(always)]
pub(crate) fn extract(words: &[u64], bit: usize, w: u32) -> u64 {
    let k = bit >> 6;
    let o = (bit & 63) as u32;
    // SAFETY: every writer pads one word past the last value, so k + 1 is in bounds.
    let (lo, hi) = unsafe { (*words.get_unchecked(k), *words.get_unchecked(k + 1)) };
    // `(hi << 1) << (63 - o)` is `hi << (64 - o)` without the shift by 64 when o is 0.
    ((lo >> o) | ((hi << 1) << (63 - o))) & mask(w)
}

pub(crate) fn put(words: &mut Vec<u64>, bit: usize, w: u32, v: u64) {
    while words.len() < (bit + w as usize) / 64 + 2 {
        words.push(0);
    }
    if w == 0 {
        return;
    }
    let k = bit >> 6;
    let o = bit & 63;
    words[k] |= v << o;
    if o + w as usize > 64 {
        words[k + 1] |= v >> (64 - o);
    }
}

/// `put` into words already long enough (one word past the value).
#[inline(always)]
fn or_in(words: &mut [u64], bit: usize, w: u32, v: u64) {
    if w == 0 {
        return;
    }
    let k = bit >> 6;
    let o = bit & 63;
    words[k] |= v << o;
    // `(v >> 1) >> (63 - o)` is `v >> (64 - o)` without the shift by 64 when o is 0.
    words[k + 1] |= (v >> 1) >> (63 - o);
}

pub(crate) fn bits_for(v: u64) -> u32 {
    64 - v.leading_zeros()
}

/// One fixed width over the whole array.
pub struct Packed {
    pub w: u32,
    pub words: Vec<u64>,
}

impl Image for Packed {
    fn image(&self, out: &mut Vec<u8>) {
        out.push(self.w as u8);
        raw(out, &self.words);
    }
}

impl Packed {
    pub fn new(vals: &[u64]) -> Packed {
        let w = vals.iter().map(|&v| bits_for(v)).max().unwrap_or(0);
        let mut words = vec![0u64; 2];
        for (i, &v) in vals.iter().enumerate() {
            put(&mut words, i * w as usize, w, v);
        }
        Packed { w, words }
    }
    /// At width `w` exactly (every value below 2^w).
    pub fn new_width(vals: &[u64], w: u32) -> Packed {
        let mut words = vec![0u64; 2];
        for (i, &v) in vals.iter().enumerate() {
            put(&mut words, i * w as usize, w, v);
        }
        Packed { w, words }
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        extract(&self.words, i * self.w as usize, self.w)
    }
    pub fn bytes(&self) -> usize {
        self.words.len() * 8 + 1
    }
    /// The codes from 0 to n, 64 at a time: `f(base, m)` with bit j of m set when value base + j is in `set`
    /// (indexed by value). 64 values at width w fill exactly w words, so each group is w aligned words read
    /// with constant shifts: the loop is compiled once per width, as generated code would be.
    pub fn match_groups(&self, n: usize, set: &[bool], f: impl FnMut(usize, u64)) {
        macro_rules! by_width {
            ($($w:literal)*) => {
                match self.w {
                    $($w => match_w::<$w>(&self.words, n, set, f),)*
                    _ => match_any(self, n, set, f),
                }
            };
        }
        by_width!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)
    }
}

/// `$body` once for each j in 0..64, with j a constant in each copy.
macro_rules! unroll64 {
    ($j:ident, $body:block) => {
        unroll64!(@ $j, $body, 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
            32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63)
    };
    (@ $j:ident, $body:block, $($n:literal)*) => {
        $({ const $j: usize = $n; $body })*
    };
}

#[inline(always)]
fn match_w<const W: usize>(words: &[u64], n: usize, set: &[bool], mut f: impl FnMut(usize, u64)) {
    let full = n / 64;
    let sp = set.as_ptr();
    for g in 0..full {
        // SAFETY: group g's words are g·W .. g·W + W, inside the array (n values at W bits, one word of padding).
        let gw = unsafe { words.as_ptr().add(g * W) };
        let mut m = 0u64;
        unroll64!(J, {
            let (k, o) = (J * W >> 6, J * W & 63);
            // SAFETY: k + 1 ≤ W, inside the group's words and the padding word.
            let v = unsafe {
                if o + W <= 64 { *gw.add(k) >> o } else { (*gw.add(k) >> o) | (*gw.add(k + 1) << ((64 - o) & 63)) }
            } & ((1u64 << W) - 1);
            // SAFETY: every code indexes the set (the set covers the language's codes).
            m |= u64::from(unsafe { *sp.add(v as usize) }) << J;
        });
        f(g * 64, m);
    }
    let rest = n - full * 64;
    if rest > 0 {
        let mut m = 0u64;
        for j in 0..rest {
            let v = extract(words, (full * 64 + j) * W, W as u32);
            m |= u64::from(set[v as usize]) << j;
        }
        f(full * 64, m);
    }
}

fn match_any(p: &Packed, n: usize, set: &[bool], mut f: impl FnMut(usize, u64)) {
    for g in (0..n).step_by(64) {
        let mut m = 0u64;
        for j in 0..(n - g).min(64) {
            m |= u64::from(set[p.get(g + j) as usize]) << j;
        }
        f(g, m);
    }
}

/// A block of `BLOCK` values at a time: a base (the block's least value, if kept), a width `w`, and the values
/// above the base at `w` bits. A block's top `e` codes are escapes: a slot at or above `2^w − e` names its
/// exception directly (no search), and the exceptions follow the block's slots at the block's exception
/// width. A block's header is one word: `w` (6 bits), the exception width (6), `e` (9), the bit offset (43).
pub struct PFor {
    n: usize,
    head: Vec<u64>,
    base: Vec<u64>,
    words: Vec<u64>,
    with_base: bool,
}

impl Image for PFor {
    fn image(&self, out: &mut Vec<u8>) {
        raw(out, &self.head);
        if self.with_base {
            raw(out, &self.base);
        }
        raw(out, &self.words);
    }
}

/// A block's (w, e, ew): `w` bits a slot, `e` escapes, exceptions at `ew` bits. From a histogram of the
/// values' widths, `w` is the one costing least if the escapes were just the values of more than `w` bits;
/// then `e` is made exact for that `w` (the values at or above 2^w − e), one pass at a time.
fn esc_width(rel: &[u64]) -> (u32, u32, u32) {
    let mut hist = [0u32; 65];
    for &v in rel {
        hist[bits_for(v) as usize] += 1;
    }
    let top = (0..=64).rev().find(|&b| hist[b] > 0).unwrap_or(0) as u32;
    let mut best = (u64::MAX, top);
    let mut above = 0u64;
    for w in (0..=top).rev() {
        // `above`: the values of more than `w` bits.
        let cost = rel.len() as u64 * u64::from(w) + above * u64::from(top);
        if above <= (1u64 << w) && cost <= best.0 {
            best = (cost, w);
        }
        above += u64::from(hist[w as usize]);
    }
    let mut w = best.1;
    loop {
        let codes = 1u64 << w;
        let mut e = rel.iter().filter(|&&v| v >= codes).count() as u64;
        loop {
            let ne = rel.iter().filter(|&&v| v >= codes.saturating_sub(e)).count() as u64;
            if ne == e || ne > codes {
                e = ne;
                break;
            }
            e = ne;
        }
        if e <= codes && e <= BLOCK as u64 {
            let ew = if e == 0 { 0 } else { top };
            return (w, e as u32, ew);
        }
        w += 1;
    }
}

impl PFor {
    pub fn new(vals: &[u64], with_base: bool) -> PFor {
        let mut p = PFor { n: vals.len(), head: vec![], base: vec![], words: vec![0; 2], with_base };
        let mut bit = 0usize;
        for chunk in vals.chunks(BLOCK) {
            let b = if with_base { *chunk.iter().min().unwrap() } else { 0 };
            let rel: Vec<u64> = chunk.iter().map(|&v| v - b).collect();
            let (w, e, ew) = esc_width(&rel);
            assert!(w < 64 && ew < 64 && (bit as u64) < 1 << 43);
            p.head.push(u64::from(w) | u64::from(ew) << 6 | u64::from(e) << 12 | (bit as u64) << 21);
            p.base.push(b);
            let lim = (1u64 << w) - u64::from(e);
            let exc_at = bit + rel.len() * w as usize;
            let end = exc_at + e as usize * ew as usize;
            p.words.resize(end / 64 + 2, 0);
            let mut k = 0u64;
            for &v in &rel {
                if v >= lim {
                    or_in(&mut p.words, bit, w, lim + k);
                    or_in(&mut p.words, exc_at + k as usize * ew as usize, ew, v);
                    k += 1;
                } else {
                    or_in(&mut p.words, bit, w, v);
                }
                bit += w as usize;
            }
            bit = end;
        }
        put(&mut p.words, bit, 0, 0);
        p
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        let b = i / BLOCK;
        // SAFETY: i is within the array the reader was built over.
        let h = unsafe { *self.head.get_unchecked(b) };
        let (w, e, off) = ((h & 63) as u32, (h >> 12) & 511, (h >> 21) as usize);
        let v = extract(&self.words, off + (i % BLOCK) * w as usize, w);
        let base = if self.with_base { unsafe { *self.base.get_unchecked(b) } } else { 0 };
        let lim = (1u64 << w) - e;
        if v >= lim {
            let ew = ((h >> 6) & 63) as u32;
            let len = (self.n - b * BLOCK).min(BLOCK);
            return base + extract(&self.words, off + len * w as usize + (v - lim) as usize * ew as usize, ew);
        }
        base + v
    }
    /// Value i as stored: (its slot's width, the slot's bits, the exception's bits if the slot is an escape).
    /// Two runs of values are the same bits (up to a shift) when their fields are equal.
    pub fn field(&self, i: usize) -> (u32, u64, Option<u64>) {
        let b = i / BLOCK;
        let h = self.head[b];
        let (w, ew, e, off) = ((h & 63) as u32, ((h >> 6) & 63) as u32, (h >> 12) & 511, (h >> 21) as usize);
        let v = extract(&self.words, off + (i % BLOCK) * w as usize, w);
        let lim = (1u64 << w) - e;
        let len = (self.n - b * BLOCK).min(BLOCK);
        let exc = (v >= lim).then(|| extract(&self.words, off + len * w as usize + (v - lim) as usize * ew as usize, ew));
        (w, v, exc)
    }
    /// Block b's header, read once, for reading the block's values in order.
    #[inline(always)]
    pub fn block(&self, b: usize) -> PBlock<'_> {
        let h = self.head[b];
        let (w, ew, e, off) = ((h & 63) as u32, ((h >> 6) & 63) as u32, (h >> 12) & 511, (h >> 21) as usize);
        let len = (self.n - b * BLOCK).min(BLOCK);
        PBlock { words: &self.words, off, w, lim: (1u64 << w) - e, ew, exc: off + len * w as usize, base: if self.with_base { self.base[b] } else { 0 } }
    }
    pub fn bytes(&self) -> usize {
        self.head.len() * 8 + if self.with_base { self.base.len() * 8 } else { 0 } + self.words.len() * 8 + 8
    }
}

/// One `PFor` block with its header decoded: value j is one extract, or two for an exception.
#[derive(Clone, Copy)]
pub struct PBlock<'a> {
    words: &'a [u64],
    off: usize,
    w: u32,
    lim: u64,
    ew: u32,
    exc: usize,
    base: u64,
}

impl PBlock<'_> {
    #[inline(always)]
    pub fn get(&self, j: usize) -> u64 {
        let v = extract(self.words, self.off + j * self.w as usize, self.w);
        if v >= self.lim {
            self.base + extract(self.words, self.exc + (v - self.lim) as usize * self.ew as usize, self.ew)
        } else {
            self.base + v
        }
    }
}

/// The language's codes: each distinct (kind, field, flags, classes) once, as a grammar numbering would.
#[derive(Default)]
pub struct Codes {
    pub ids: std::collections::HashMap<(u16, u8, u8, Vec<u16>), u32>,
    pub table: Vec<(u16, u8, u8, Vec<u16>)>,
}

impl Codes {
    pub fn learn(&mut self, tree: &Tree) {
        for n in &tree.nodes {
            let k = (n.kind, n.field, n.flags, n.classes.clone());
            if !self.ids.contains_key(&k) {
                self.ids.insert(k.clone(), self.table.len() as u32);
                self.table.push(k);
            }
        }
    }
    pub fn width(&self) -> u32 {
        bits_for(self.table.len().saturating_sub(1) as u64).max(1)
    }
}

/// What a candidate answers. Node ids are the candidate's own (`id_of` maps a preorder number to one).
pub trait Reader {
    fn bytes(&self) -> usize;
    fn id_of(&self, preorder: usize) -> usize;
    /// The code's index into the language's table (format 15: its tape's table, mapped by `kind`).
    fn code(&self, id: usize) -> u32;
    /// The id just past the subtree.
    fn subtree_end(&self, id: usize) -> usize;
    fn parent(&self, id: usize) -> Option<usize>;
    fn start(&self, id: usize) -> u64;
    fn end(&self, id: usize) -> u64;
    /// The first child, if any.
    fn first_child(&self, id: usize) -> Option<usize>;
    /// The next sibling of `id`, a child of a node whose subtree ends at `parent_end`.
    fn next_sibling(&self, id: usize, parent_end: usize) -> Option<usize>;
    /// The node after `id` in preorder (a sweep's step), if any; the first node is `id_of(0)`.
    fn next(&self, id: usize) -> Option<usize>;
    /// The number of nodes.
    fn len(&self) -> usize;
    /// The node before `id` in preorder, if any.
    fn prev(&self, id: usize) -> Option<usize>;
    /// A node's preorder number (its rank), and the code by rank: what a block summary is indexed by.
    fn rank(&self, id: usize) -> usize;
    fn code_by_rank(&self, r: usize) -> u32;
    /// The codes by rank as one packed array, when the candidate keeps them so (a sweep then reads them 64 at a
    /// time).
    fn packed_codes(&self) -> Option<&Packed> {
        None
    }
    /// See `Pos::end_from_next_start`.
    fn end_from_next_start(&self, _id: usize, _next_start: u64) -> Option<u64> {
        None
    }
    /// Every node's span, in preorder, a block at a time; false if the candidate has no block reader.
    fn spans_bulk(&self, _out: &mut Vec<(u64, u64)>) -> bool {
        false
    }
    /// Ids are preorder numbers (rank is the identity), so a node is found from its rank.
    const PREORDER_IDS: bool;
}

// ---- F15: format 15 in place ----------------------------------------------------------------------

pub struct F15<'a> {
    n: usize,
    code: &'a [u8],
    size: &'a [u8],
    base: &'a [u8],
    off: &'a [u8],
    tail: &'a [u8],
    w: [usize; 3],
    from_start: bool,
    /// The tape's code number to the language's (for checks).
    to_lang: Vec<u32>,
    image_bytes: usize,
}

#[inline(always)]
fn le(col: &[u8], width: usize, i: usize) -> u64 {
    // SAFETY: format 15 pads every column to 16 bytes, so an 8-byte read at a value stays in the image.
    let v = unsafe { std::ptr::read_unaligned(col.as_ptr().add(i * width) as *const u64) };
    v & mask(width as u32 * 8)
}

impl<'a> F15<'a> {
    pub fn new(image: &'a [u8], codes: &Codes) -> F15<'a> {
        use rerail_tape_format::{CHECKPOINT_STRIDE, HEADER_BYTES, TAIL_FROM_START, header, padded};
        let u64_at = |at: usize| u64::from_le_bytes(image[at..at + 8].try_into().unwrap()) as usize;
        let w = [image[header::WIDTHS] as usize, image[header::WIDTHS + 1] as usize, (image[header::WIDTHS + 2] & !TAIL_FROM_START) as usize];
        let from_start = image[header::WIDTHS + 2] & TAIL_FROM_START != 0;
        let (n, ncodes) = (u64_at(header::NODES), u64_at(header::CODES));
        let mut at = HEADER_BYTES;
        let mut column = |width: usize, count: usize| {
            let c = &image[at..];
            at = padded(at + width * count);
            c
        };
        let code = column(2, n);
        let size = column(w[0], n);
        let base = column(4, n.div_ceil(CHECKPOINT_STRIDE));
        let off = column(w[1], n);
        let tail = column(w[2], n);
        let mut to_lang = Vec::new();
        let mut c = at;
        for _ in 0..ncodes {
            let kf = u16::from_le_bytes([image[c], image[c + 1]]);
            let flags = image[c + 2];
            let alias = u16::from_le_bytes([image[c + 3], image[c + 4]]);
            let count = image[c + 5] as usize;
            let classes: Vec<u16> = (0..count).map(|k| u16::from_le_bytes([image[c + 6 + 2 * k], image[c + 7 + 2 * k]])).collect();
            let kind = if alias != u16::MAX { 0x8000 | alias } else { kf & 1023 };
            to_lang.push(codes.ids.get(&(kind, (kf >> 10) as u8, flags, classes)).copied().unwrap_or(u32::MAX));
            c += 6 + 2 * count;
        }
        F15 { n, code, size, base, off, tail, w, from_start, to_lang, image_bytes: image.len() }
    }
    #[inline(always)]
    fn size(&self, i: usize) -> usize {
        le(self.size, self.w[0], i) as usize
    }
}

impl Reader for F15<'_> {
    fn bytes(&self) -> usize {
        self.image_bytes
    }
    fn id_of(&self, p: usize) -> usize {
        p
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.to_lang[le(self.code, 2, i) as usize]
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + self.size(i).max(1)
    }
    /// Not stored: back over the preceding nodes to the first whose subtree holds `i`.
    fn parent(&self, i: usize) -> Option<usize> {
        let mut j = i;
        while j > 0 {
            j -= 1;
            if j + self.size(j) > i {
                return Some(j);
            }
        }
        None
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        le(self.base, 4, i / rerail_tape_format::CHECKPOINT_STRIDE) + le(self.off, self.w[1], i)
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        let last = i + self.size(i).max(1) - 1;
        let from = if self.from_start { self.start(i) } else { self.start(last) };
        from.wrapping_add(le(self.tail, self.w[2], i)) & 0xffff_ffff
    }
    #[inline(always)]
    fn first_child(&self, i: usize) -> Option<usize> {
        (self.size(i) > 1).then_some(i + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, i: usize, pe: usize) -> Option<usize> {
        let j = self.subtree_end(i);
        (j < pe && j < self.n).then_some(j)
    }
    #[inline(always)]
    fn next(&self, id: usize) -> Option<usize> {
        (id + 1 < Reader::len(self)).then_some(id + 1)
    }
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn prev(&self, id: usize) -> Option<usize> {
        id.checked_sub(1)
    }
    #[inline(always)]
    fn rank(&self, id: usize) -> usize {
        id
    }
    #[inline(always)]
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code(r)
    }
    const PREORDER_IDS: bool = true;
}

// ---- X: explicit, bit-packed ---------------------------------------------------------------------

/// Each node's code at the language's width.
/// What every builder reads, in preorder: flat columns, filled from a tree (`of_tree`) or straight from
/// format 16's events (`of_events`, shape 2's loading). Scratch for the build, dropped after it.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct Cols {
    pub code: Vec<u64>,
    pub start: Vec<u64>,
    pub end: Vec<u64>,
    pub size: Vec<u32>,
    pub parent: Vec<u32>,
    pub source_len: u64,
}

impl Cols {
    pub fn len(&self) -> usize {
        self.code.len()
    }
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }
    pub fn of_tree(tree: &Tree, codes: &Codes) -> Cols {
        Cols {
            code: tree.nodes.iter().map(|n| u64::from(codes.ids[&(n.kind, n.field, n.flags, n.classes.clone())])).collect(),
            start: tree.nodes.iter().map(|n| n.start).collect(),
            end: tree.nodes.iter().map(|n| n.end).collect(),
            size: tree.size.clone(),
            parent: tree.parent.clone(),
            source_len: tree.source_len,
        }
    }
}

/// The language's codes keyed as format 16's events carry them: (kind, field, flags, membership mask).
pub struct CodeMap(std::collections::HashMap<u64, u32, std::hash::BuildHasherDefault<Mix>>);

/// A one-multiply hash for the code map's u64 keys.
#[derive(Default)]
pub struct Mix(u64);

impl std::hash::Hasher for Mix {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("only u64 keys")
    }
    fn write_u64(&mut self, k: u64) {
        self.0 = (k ^ (k >> 29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

fn event_key(kind: u16, field: u8, flags: u8, memb: u16) -> u64 {
    u64::from(kind) | u64::from(field) << 16 | u64::from(flags) << 24 | u64::from(memb) << 32
}

impl CodeMap {
    pub fn new(codes: &Codes, tag: u8, cands: &crate::events::Candidates) -> CodeMap {
        let mut m = std::collections::HashMap::default();
        for (id, (kind, field, flags, classes)) in codes.table.iter().enumerate() {
            let node = crate::tree::Node { kind: *kind, field: *field, flags: *flags, classes: classes.clone(), start: 0, end: 0 };
            m.insert(event_key(*kind, *field, *flags, cands.mask(tag, &node)), id as u32);
        }
        CodeMap(m)
    }
}

impl Cols {
    /// Shape 2's build input straight from the events: one pass with a stack of open nodes, no tree.
    pub fn of_events(ev: &crate::events::Events, codes: &CodeMap, source_len: u64) -> Cols {
        use crate::events::apply;
        let n = ev.opens.len();
        let mut c = Cols { code: Vec::with_capacity(n), start: Vec::with_capacity(n), end: vec![0; n], size: vec![0; n], parent: Vec::with_capacity(n), source_len };
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        let (mut last, mut t) = (0u64, 0usize);
        let trails = &ev.trails;
        let mut close = |stack: &mut Vec<u32>, c: &mut Cols, last: &mut u64, at: usize| {
            let j = stack.pop().expect("a close with a node open") as usize;
            *last = apply(*last, trails[t]);
            t += 1;
            c.end[j] = *last;
            c.size[j] = (at - j) as u32;
        };
        for (i, o) in ev.opens.iter().enumerate() {
            for _ in 0..o.pops {
                close(&mut stack, &mut c, &mut last, i);
            }
            last = apply(last, o.lead);
            c.start.push(last);
            c.parent.push(stack.last().copied().unwrap_or(NONE));
            c.code.push(u64::from(codes.0[&event_key(o.kind, o.field, o.flags, o.memb)]));
            stack.push(i as u32);
        }
        for _ in 0..ev.end_pops {
            close(&mut stack, &mut c, &mut last, n);
        }
        c
    }
}

impl Cols {
    /// The same, read straight off format 16's byte planes (`layout::Layout::BytePlanes`, no extras) as
    /// zstd gives them back: each field's bytes at its plane, no events built between.
    pub fn of_planes(bytes: &[u8], codes: &CodeMap, source_len: u64) -> Cols {
        use crate::events::apply;
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let (n, m, end_pops) = (u32_at(0) as usize, u32_at(4) as usize, u32_at(8));
        // The planes: kind (2), field, flags, memb (2), lead (4), pops (4); then the trails (4).
        let plane = |k: usize| &bytes[16 + k * n..16 + (k + 1) * n];
        let (k0, k1, fld, flg, m0, m1) = (plane(0), plane(1), plane(2), plane(3), plane(4), plane(5));
        let (l, p) = ([plane(6), plane(7), plane(8), plane(9)], [plane(10), plane(11), plane(12), plane(13)]);
        let tr = |k: usize| &bytes[16 + 14 * n + k * m..16 + 14 * n + (k + 1) * m];
        let t4 = [tr(0), tr(1), tr(2), tr(3)];
        let le4 = |pl: &[&[u8]; 4], i: usize| u32::from(pl[0][i]) | u32::from(pl[1][i]) << 8 | u32::from(pl[2][i]) << 16 | u32::from(pl[3][i]) << 24;
        let mut c = Cols { code: Vec::with_capacity(n), start: Vec::with_capacity(n), end: vec![0; n], size: vec![0; n], parent: Vec::with_capacity(n), source_len };
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        let (mut last, mut t) = (0u64, 0usize);
        let mut close = |stack: &mut Vec<u32>, c: &mut Cols, last: &mut u64, at: usize| {
            let j = stack.pop().expect("a close with a node open") as usize;
            *last = apply(*last, le4(&t4, t));
            t += 1;
            c.end[j] = *last;
            c.size[j] = (at - j) as u32;
        };
        for i in 0..n {
            for _ in 0..le4(&p, i) {
                close(&mut stack, &mut c, &mut last, i);
            }
            last = apply(last, le4(&l, i));
            c.start.push(last);
            c.parent.push(stack.last().copied().unwrap_or(NONE));
            let kind = u16::from(k0[i]) | u16::from(k1[i]) << 8;
            let memb = u16::from(m0[i]) | u16::from(m1[i]) << 8;
            c.code.push(u64::from(codes.0[&event_key(kind, fld[i], flg[i], memb)]));
            stack.push(i as u32);
        }
        for _ in 0..end_pops {
            close(&mut stack, &mut c, &mut last, n);
        }
        c
    }
}

pub fn code_of(c: &Cols) -> Packed {
    Packed::new(&c.code)
}

/// How a candidate stores start and end, by preorder number `r`. `next` gives the preorder number just past
/// the node's subtree (n past the last), `post` its postorder number; each is asked only if the form needs it.
pub trait Pos: Sized + Image {
    const NAME: &'static str;
    fn build(c: &Cols) -> Self;
    fn bytes(&self) -> [usize; 2];
    fn start(&self, r: usize) -> u64;
    fn end(&self, r: usize, next: impl FnOnce() -> usize, post: impl FnOnce() -> usize) -> u64;
    /// The end of node r, given the start of the node just past its subtree, when the form keeps the end as
    /// the gap to that start (a sweep in preorder reaches that start itself).
    #[inline(always)]
    fn end_from_next_start(&self, _r: usize, _next_start: u64) -> Option<u64> {
        None
    }
    /// The end is kept as a gap to the next start (`scan_block` then gives the gap, zigzagged).
    const GAP_END: bool = false;
    /// Block b's nodes in order, `f(j, start, x)` with x the end, or the zigzagged gap if `GAP_END`; false if
    /// the form has no block reader.
    fn scan_block(&self, _b: usize, _f: impl FnMut(usize, u64, u64)) -> bool {
        false
    }
    /// Start in preorder and end in postorder, both Elias–Fano (BE): a sweep reads both in order.
    fn ef_parts(&self) -> Option<(&Ef, &Ef)> {
        None
    }
    /// All n nodes in order, `f(i, start, x)` as `scan_block` gives them; false if the form has no reader in order.
    fn scan_all(&self, n: usize, mut f: impl FnMut(usize, u64, u64)) -> bool {
        for b in 0..n.div_ceil(BLOCK) {
            if !self.scan_block(b, |j, s, x| f(b * BLOCK + j, s, x)) {
                return false;
            }
        }
        true
    }
}

/// Start by frame of reference a block at a time, end as a length.
pub struct ForLen {
    start: PFor,
    len: PFor,
}

impl Image for ForLen {
    fn image(&self, out: &mut Vec<u8>) {
        self.start.image(out);
        self.len.image(out);
    }
}

impl Pos for ForLen {
    const NAME: &'static str = "frame of reference + length";
    fn build(c: &Cols) -> Self {
        let start = &c.start;
        let len: Vec<u64> = c.start.iter().zip(&c.end).map(|(s, e)| e - s).collect();
        ForLen { start: PFor::new(start, true), len: PFor::new(&len, false) }
    }
    fn bytes(&self) -> [usize; 2] {
        [self.start.bytes(), self.len.bytes()]
    }
    #[inline(always)]
    fn start(&self, r: usize) -> u64 {
        self.start.get(r)
    }
    #[inline(always)]
    fn end(&self, r: usize, _: impl FnOnce() -> usize, _: impl FnOnce() -> usize) -> u64 {
        self.start.get(r) + self.len.get(r)
    }    #[inline(always)]
    fn scan_block(&self, b: usize, mut f: impl FnMut(usize, u64, u64)) -> bool {
        let (st, ln) = (self.start.block(b), self.len.block(b));
        for j in 0..(self.start.n - b * BLOCK).min(BLOCK) {
            let s = st.get(j);
            f(j, s, s + ln.get(j));
        }
        true
    }
}

/// Start by mldb's predictor and residual (a line a block, or none where that is smaller), end as the gap
/// to the start of the node just past the subtree (the source's length past the last).
pub struct PredGap {
    start: Pred,
    gap: PFor,
    source_len: u64,
}

impl Image for PredGap {
    fn image(&self, out: &mut Vec<u8>) {
        self.start.image(out);
        self.gap.image(out);
    }
}

impl Pos for PredGap {
    const NAME: &'static str = "predictor + residual, end as the gap to the next start";
    fn build(c: &Cols) -> Self {
        let n = c.len();
        let start = &c.start;
        let gap: Vec<u64> = (0..n).map(|i| {
            let next = i + c.size[i] as usize;
            let s = if next < n { start[next] } else { c.source_len };
            let d = s as i64 - c.end[i] as i64;
            ((d << 1) ^ (d >> 63)) as u64
        }).collect();
        PredGap { start: Pred::new(start), gap: PFor::new(&gap, false), source_len: c.source_len }
    }
    fn bytes(&self) -> [usize; 2] {
        [self.start.bytes(), self.gap.bytes() + 8]
    }
    #[inline(always)]
    fn start(&self, r: usize) -> u64 {
        self.start.get(r)
    }
    #[inline(always)]
    fn end(&self, r: usize, next: impl FnOnce() -> usize, _: impl FnOnce() -> usize) -> u64 {
        let nx = next();
        let s = if nx < self.start.n { self.start.get(nx) } else { self.source_len };
        let z = self.gap.get(r);
        let d = ((z >> 1) as i64) ^ -((z & 1) as i64);
        (s as i64 - d) as u64
    }
    #[inline(always)]
    fn end_from_next_start(&self, r: usize, next_start: u64) -> Option<u64> {
        let z = self.gap.get(r);
        Some((next_start as i64 - (((z >> 1) as i64) ^ -((z & 1) as i64))) as u64)
    }
    const GAP_END: bool = true;
    #[inline(always)]
    fn scan_block(&self, b: usize, mut f: impl FnMut(usize, u64, u64)) -> bool {
        let (sl, base) = (u64::from(self.start.slope[b]), self.start.base[b]);
        let (res, gap) = (self.start.res.block(b), self.gap.block(b));
        for j in 0..(self.start.n - b * BLOCK).min(BLOCK) {
            f(j, base.wrapping_add((j as u64 * sl) >> 16).wrapping_add(res.get(j)), gap.get(j));
        }
        true
    }
}

/// Start by the predictor and residual, end as a length (no structure read for the end).
pub struct PredLen {
    start: Pred,
    len: PFor,
}

impl Image for PredLen {
    fn image(&self, out: &mut Vec<u8>) {
        self.start.image(out);
        self.len.image(out);
    }
}

impl Pos for PredLen {
    const NAME: &'static str = "predictor + residual, end as a length";
    fn build(c: &Cols) -> Self {
        let start = &c.start;
        let len: Vec<u64> = c.start.iter().zip(&c.end).map(|(s, e)| e - s).collect();
        PredLen { start: Pred::new(start), len: PFor::new(&len, false) }
    }
    fn bytes(&self) -> [usize; 2] {
        [self.start.bytes(), self.len.bytes()]
    }
    #[inline(always)]
    fn start(&self, r: usize) -> u64 {
        self.start.get(r)
    }
    #[inline(always)]
    fn end(&self, r: usize, _: impl FnOnce() -> usize, _: impl FnOnce() -> usize) -> u64 {
        self.start.get(r) + self.len.get(r)
    }
}

/// Start by Elias–Fano in preorder, end as the gap to the next start.
pub struct EfGap {
    start: Ef,
    n: usize,
    gap: PFor,
    source_len: u64,
}

impl Image for EfGap {
    fn image(&self, out: &mut Vec<u8>) {
        self.start.image(out);
        self.gap.image(out);
    }
}

impl Pos for EfGap {
    const NAME: &'static str = "Elias–Fano start, end as the gap to the next start";
    fn build(c: &Cols) -> Self {
        let g = PredGap::build(c);
        EfGap { start: Ef::new(&c.start, c.source_len), n: c.len(), gap: g.gap, source_len: c.source_len }
    }
    fn bytes(&self) -> [usize; 2] {
        [self.start.bytes(), self.gap.bytes() + 8]
    }
    #[inline(always)]
    fn start(&self, r: usize) -> u64 {
        self.start.get(r)
    }
    #[inline(always)]
    fn end(&self, r: usize, next: impl FnOnce() -> usize, _: impl FnOnce() -> usize) -> u64 {
        let nx = next();
        let s = if nx < self.n { self.start.get(nx) } else { self.source_len };
        let z = self.gap.get(r);
        (s as i64 - (((z >> 1) as i64) ^ -((z & 1) as i64))) as u64
    }
    #[inline(always)]
    fn end_from_next_start(&self, r: usize, next_start: u64) -> Option<u64> {
        let z = self.gap.get(r);
        Some((next_start as i64 - (((z >> 1) as i64) ^ -((z & 1) as i64))) as u64)
    }
    const GAP_END: bool = true;
    fn scan_all(&self, n: usize, mut f: impl FnMut(usize, u64, u64)) -> bool {
        let mut gb = self.gap.block(0);
        self.start.for_each(n, |i, v| {
            if i % BLOCK == 0 {
                gb = self.gap.block(i / BLOCK);
            }
            f(i, v, gb.get(i % BLOCK));
        });
        true
    }
}

/// Start by Elias–Fano in preorder, end by Elias–Fano in postorder (ends never decrease there).
pub struct EfEf {
    start: Ef,
    end: Ef,
}

impl Image for EfEf {
    fn image(&self, out: &mut Vec<u8>) {
        self.start.image(out);
        self.end.image(out);
    }
}

impl Pos for EfEf {
    const NAME: &'static str = "Elias–Fano: start in preorder, end in postorder";
    fn build(c: &Cols) -> Self {
        let n = c.len();
        // Postorder: a node after its subtree.
        let mut post = Vec::with_capacity(n);
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..n {
            while let Some(&top) = stack.last() {
                if top + c.size[top] as usize <= i {
                    post.push(c.end[top]);
                    stack.pop();
                } else {
                    break;
                }
            }
            stack.push(i);
        }
        while let Some(top) = stack.pop() {
            post.push(c.end[top]);
        }
        EfEf { start: Ef::new(&c.start, c.source_len), end: Ef::new(&post, c.source_len) }
    }
    fn bytes(&self) -> [usize; 2] {
        [self.start.bytes(), self.end.bytes()]
    }
    #[inline(always)]
    fn start(&self, r: usize) -> u64 {
        self.start.get(r)
    }
    #[inline(always)]
    fn end(&self, _: usize, _: impl FnOnce() -> usize, post: impl FnOnce() -> usize) -> u64 {
        self.end.get(post())
    }
    fn ef_parts(&self) -> Option<(&Ef, &Ef)> {
        Some((&self.start, &self.end))
    }
}

/// Values a block at a time as `base + (j * slope >> 16) + residual`, the residual bit-packed with patched
/// exceptions; slope 0 (plain frame of reference) where that is smaller (mldb's choice a range at a time).
pub struct Pred {
    n: usize,
    slope: Vec<u32>,
    res: PFor,
    base: Vec<u64>,
}

impl Image for Pred {
    fn image(&self, out: &mut Vec<u8>) {
        raw(out, &self.slope);
        raw(out, &self.base);
        self.res.image(out);
    }
}

impl Pred {
    pub fn new(vals: &[u64]) -> Pred {
        let (mut slope, mut base, mut res) = (Vec::new(), Vec::new(), Vec::new());
        res.reserve(vals.len());
        for chunk in vals.chunks(BLOCK) {
            let at = |j: usize, sl: u64| chunk[j] as i64 - ((j as u64 * sl) >> 16) as i64;
            // The residuals' least value and the bits above it, for a slope.
            let fit = |sl: u64| -> (i64, u32) {
                let (mut lo, mut hi) = (i64::MAX, i64::MIN);
                for j in 0..chunk.len() {
                    let r = at(j, sl);
                    lo = lo.min(r);
                    hi = hi.max(r);
                }
                (lo, bits_for((hi - lo) as u64))
            };
            let span = chunk.last().unwrap() - chunk[0];
            let line = if chunk.len() > 1 { (span << 16) / (chunk.len() as u64 - 1) } else { 0 };
            let line = line.min(u64::from(u32::MAX));
            let (b0, c0) = fit(0);
            let (b1, c1) = fit(line);
            let (sl, b) = if c1 < c0 { (line, b1) } else { (0, b0) };
            slope.push(sl as u32);
            base.push(b as u64);
            res.extend((0..chunk.len()).map(|j| (at(j, sl) - b) as u64));
        }
        Pred { n: vals.len(), slope, res: PFor::new(&res, false), base }
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        let b = i / BLOCK;
        // SAFETY: i is within the array.
        let (sl, base) = unsafe { (*self.slope.get_unchecked(b), *self.base.get_unchecked(b)) };
        base.wrapping_add(((i % BLOCK) as u64 * u64::from(sl)) >> 16).wrapping_add(self.res.get(i))
    }
    pub fn bytes(&self) -> usize {
        self.slope.len() * 4 + self.base.len() * 8 + self.res.bytes()
    }
}

/// Elias–Fano over a non-decreasing sequence below `universe`: `l` low bits a value, packed; the high
/// parts in unary (value i's one at `high(i) + i`), with the position of every `EF_SAMPLE`-th one.
pub struct Ef {
    l: u32,
    low: Packed,
    high: Vec<u64>,
    samples: Vec<u32>,
}

pub const EF_SAMPLE: usize = 64;

pub struct EfCursor<'a> {
    ef: &'a Ef,
    w: usize,
    word: u64,
    i: usize,
}

impl EfCursor<'_> {
    /// The next value (the caller asks no more than the list holds).
    #[inline(always)]
    pub fn next(&mut self) -> u64 {
        while self.word == 0 {
            self.w += 1;
            // SAFETY: the list holds another value, so another one bit lies ahead.
            self.word = unsafe { *self.ef.high.get_unchecked(self.w) };
        }
        let p = self.w * 64 + self.word.trailing_zeros() as usize;
        self.word &= self.word - 1;
        let v = (((p - self.i) as u64) << self.ef.l) | self.ef.low.get(self.i);
        self.i += 1;
        v
    }
}

static SELECT8: std::sync::LazyLock<[[u8; 8]; 256]> = std::sync::LazyLock::new(|| {
    let mut t = [[8u8; 8]; 256];
    for b in 0..256usize {
        let mut k = 0;
        for bit in 0..8 {
            if b >> bit & 1 == 1 {
                t[b][k] = bit as u8;
                k += 1;
            }
        }
    }
    t
});

/// The position of the `r`-th one (from 0) in `x`, which has more than `r` ones, with no branch: byte counts
/// by broadword steps, the byte by a comparison of all eight at once, then a table (Vigna, "Broadword
/// implementation of rank/select queries").
#[cfg(not(all(target_arch = "x86_64", target_feature = "bmi2")))]
#[inline(always)]
fn select_in_word(x: u64, r: u32) -> u32 {
    const L8: u64 = 0x0101_0101_0101_0101;
    const H8: u64 = 0x8080_8080_8080_8080;
    let mut s = x - ((x >> 1) & 0x5555_5555_5555_5555);
    s = (s & 0x3333_3333_3333_3333) + ((s >> 2) & 0x3333_3333_3333_3333);
    s = (s + (s >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    s = s.wrapping_mul(L8);
    let geq = ((u64::from(r) * L8 | H8) - s) & H8;
    let place = geq.count_ones() * 8;
    let rank = u64::from(r) - (((s << 8) >> place) & 0xff);
    // SAFETY: the byte and the rank within it index the 256 × 8 table.
    place + u32::from(unsafe { *SELECT8.get_unchecked(((x >> place) & 0xff) as usize).get_unchecked(rank as usize) })
}

/// x86 with BMI2: the r-th one is where PDEP deposits a single bit (fast from Haswell and Zen 3; Zen 1
/// and 2 run PDEP in microcode, so their builds leave BMI2 off and take the broadword select).
#[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
#[inline(always)]
fn select_in_word(x: u64, r: u32) -> u32 {
    // SAFETY: the build enables BMI2.
    unsafe { core::arch::x86_64::_pdep_u64(1u64 << r, x).trailing_zeros() }
}

impl Image for Ef {
    fn image(&self, out: &mut Vec<u8>) {
        out.push(self.l as u8);
        self.low.image(out);
        raw(out, &self.high);
        raw(out, &self.samples);
    }
}

impl Ef {
    pub fn new(vals: &[u64], universe: u64) -> Ef {
        let n = vals.len().max(1) as u64;
        let l = if universe > n { 63 - (universe / n).leading_zeros() } else { 0 };
        let low: Vec<u64> = vals.iter().map(|&v| v & mask(l)).collect();
        let hbits = (universe >> l) as usize + vals.len() + 2;
        let mut high = vec![0u64; hbits.div_ceil(64) + 5];
        let mut samples = Vec::new();
        for (i, &v) in vals.iter().enumerate() {
            let p = (v >> l) as usize + i;
            high[p >> 6] |= 1 << (p & 63);
            if i % EF_SAMPLE == 0 {
                samples.push(p as u32);
            }
        }
        Ef { l, low: Packed::new_width(&low, l), high, samples }
    }
    #[inline(always)]
    /// The values from the first, one at a time: the cursor's word of high bits, its ones taken by trailing
/// zeros.
    pub fn cursor(&self) -> EfCursor<'_> {
        EfCursor { ef: self, w: 0, word: self.high[0], i: 0 }
    }
    /// The first n values in order: the high bits' ones walked a word at a time, each low part read in place.
    pub fn for_each(&self, n: usize, mut f: impl FnMut(usize, u64)) {
        let mut i = 0usize;
        for (w, &h) in self.high.iter().enumerate() {
            let mut word = h;
            while word != 0 && i < n {
                let p = w * 64 + word.trailing_zeros() as usize;
                f(i, (((p - i) as u64) << self.l) | self.low.get(i));
                i += 1;
                word &= word - 1;
            }
            if i >= n {
                break;
            }
        }
    }
    pub fn get(&self, i: usize) -> u64 {
        // The (i % EF_SAMPLE)-th one after the sampled one: four words' counts at once, the word chosen by
        // comparisons (no branch), a loop only when the one lies further (a long gap in the values).
        // SAFETY: the high bits are padded by two words past the last one, and four words are read only where
        // they exist (`high` gets four words of padding).
        let p = unsafe { *self.samples.get_unchecked(i / EF_SAMPLE) } as usize;
        let mut r = (i % EF_SAMPLE) as u32;
        let w0 = p >> 6;
        let h = |k: usize| unsafe { *self.high.get_unchecked(w0 + k) };
        let first = h(0) & !((1u64 << (p & 63)) - 1);
        let (c0, c1, c2) = (first.count_ones(), h(1).count_ones(), h(2).count_ones());
        let (s1, s2, s3) = (c0, c0 + c1, c0 + c1 + c2);
        let k = u32::from(r >= s1) + u32::from(r >= s2) + u32::from(r >= s3);
        let (mut w, mut word);
        if r < s3 + h(3).count_ones() {
            w = w0 + k as usize;
            word = if k == 0 { first } else { h(k as usize) };
            r -= [0, s1, s2, s3][k as usize];
        } else {
            r -= s3 + h(3).count_ones();
            w = w0 + 4;
            word = self.high[w];
            loop {
                let c = word.count_ones();
                if r < c {
                    break;
                }
                r -= c;
                w += 1;
                word = self.high[w];
            }
        }
        let pos = w * 64 + select_in_word(word, r) as usize;
        (((pos - i) as u64) << self.l) | self.low.get(i)
    }
    pub fn bytes(&self) -> usize {
        self.low.bytes() + self.high.len() * 8 + self.samples.len() * 4 + 1
    }
}

// ---- X: explicit, bit-packed ---------------------------------------------------------------------

pub struct X<P: Pos> {
    n: usize,
    code: Packed,
    size: PFor,
    back: PFor,
    pos: P,
}

impl<P: Pos> X<P> {
    pub fn new(tree: &Tree, codes: &Codes) -> X<P> {
        X::of(&Cols::of_tree(tree, codes))
    }
    pub fn of(c: &Cols) -> X<P> {
        let size: Vec<u64> = c.size.iter().map(|&s| u64::from(s)).collect();
        let back: Vec<u64> = c.parent.iter().enumerate().map(|(i, &p)| if p == NONE { 0 } else { (i - p as usize) as u64 }).collect();
        X { n: c.len(), code: code_of(c), size: PFor::new(&size, false), back: PFor::new(&back, false), pos: P::build(c) }
    }
    /// Bytes by part: code, structure, start, end.
    pub fn parts(&self) -> [usize; 4] {
        let [s, e] = self.pos.bytes();
        [self.code.bytes(), self.size.bytes() + self.back.bytes(), s, e]
    }
}

impl<P: Pos> X<P> {
    pub fn image(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.code.image(&mut out);
        self.size.image(&mut out);
        self.back.image(&mut out);
        self.pos.image(&mut out);
        out
    }
}

impl<P: Pos> Reader for X<P> {
    fn bytes(&self) -> usize {
        self.parts().iter().sum()
    }
    fn id_of(&self, p: usize) -> usize {
        p
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.code.get(i) as u32
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + self.size.get(i) as usize
    }
    #[inline(always)]
    fn parent(&self, i: usize) -> Option<usize> {
        let b = self.back.get(i) as usize;
        (b != 0).then(|| i - b)
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        self.pos.start(i)
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        self.pos.end(i, || i + self.size.get(i) as usize, || unreachable!("X has no postorder"))
    }
    #[inline(always)]
    fn first_child(&self, i: usize) -> Option<usize> {
        (self.size.get(i) > 1).then_some(i + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, i: usize, pe: usize) -> Option<usize> {
        let j = self.subtree_end(i);
        (j < pe && j < self.n).then_some(j)
    }
    #[inline(always)]
    fn next(&self, id: usize) -> Option<usize> {
        (id + 1 < Reader::len(self)).then_some(id + 1)
    }
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn prev(&self, id: usize) -> Option<usize> {
        id.checked_sub(1)
    }
    #[inline(always)]
    fn rank(&self, id: usize) -> usize {
        id
    }
    #[inline(always)]
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code(r)
    }
    const PREORDER_IDS: bool = true;
}

// ---- XF: explicit, folded ------------------------------------------------------------------------

/// X with size and back folded into one array, a slot a node, tagged by its low bit:
///
/// - an inner node's slot is its subtree size (tag 0);
/// - a leaf's slot is the back distance of the node just after it (tag 1), the one node whose parent a
///   leaf can tell: the node after an inner node is its first child, so its parent is one back and needs
///   nothing stored, and the node after a leaf is a later sibling of some ancestor.
///
/// So a leaf's size (always 1) and a first child's back (always 1) are never stored, and no rank is read.
pub struct XF<P: Pos> {
    n: usize,
    code: Packed,
    slot: PFor,
    pos: P,
}

impl<P: Pos> XF<P> {
    pub fn new(tree: &Tree, codes: &Codes) -> XF<P> {
        XF::of(&Cols::of_tree(tree, codes))
    }
    pub fn of(c: &Cols) -> XF<P> {
        let n = c.len();
        let slot: Vec<u64> = (0..n).map(|i| {
            if c.size[i] > 1 {
                u64::from(c.size[i]) << 1
            } else if i + 1 < n {
                (((i + 1 - c.parent[i + 1] as usize) as u64) << 1) | 1
            } else {
                1
            }
        }).collect();
        if std::env::var_os("XF_HIST").is_some() {
            eprintln!("XFC {:?}", pack_costs(&slot));
        }
        XF { n, code: code_of(c), slot: PFor::new(&slot, false), pos: P::build(c) }
    }
    pub fn parts(&self) -> [usize; 4] {
        let [s, e] = self.pos.bytes();
        [self.code.bytes(), self.slot.bytes(), s, e]
    }
    pub fn image(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.code.image(&mut out);
        self.slot.image(&mut out);
        self.pos.image(&mut out);
        out
    }
}

impl<P: Pos> Reader for XF<P> {
    fn bytes(&self) -> usize {
        self.parts().iter().sum()
    }
    fn id_of(&self, p: usize) -> usize {
        p
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.code.get(i) as u32
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        let v = self.slot.get(i);
        // A leaf (tag 1) ends at the next node; an inner node at its size. Leaf or inner is a coin toss, so
        // no branch: the tag masks the size away and stands for the 1.
        let t = v & 1;
        i + (((v >> 1) & t.wrapping_sub(1)) | t) as usize
    }
    #[inline(always)]
    fn parent(&self, i: usize) -> Option<usize> {
        if i == 0 {
            return None;
        }
        let v = self.slot.get(i - 1);
        // After an inner node (tag 0) the parent is one back; after a leaf, the stored distance.
        let t = v & 1;
        Some(i - (((v >> 1) & t.wrapping_neg()) | (t ^ 1)) as usize)
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        self.pos.start(i)
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        self.pos.end(i, || self.subtree_end(i), || unreachable!("XF has no postorder"))
    }
    #[inline(always)]
    fn first_child(&self, i: usize) -> Option<usize> {
        (self.slot.get(i) & 1 == 0).then_some(i + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, i: usize, pe: usize) -> Option<usize> {
        let j = self.subtree_end(i);
        (j < pe && j < self.n).then_some(j)
    }
    #[inline(always)]
    fn next(&self, id: usize) -> Option<usize> {
        (id + 1 < Reader::len(self)).then_some(id + 1)
    }
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn prev(&self, id: usize) -> Option<usize> {
        id.checked_sub(1)
    }
    #[inline(always)]
    fn rank(&self, id: usize) -> usize {
        id
    }
    #[inline(always)]
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code(r)
    }
    #[inline(always)]
    fn end_from_next_start(&self, id: usize, next_start: u64) -> Option<u64> {
        self.pos.end_from_next_start(id, next_start)
    }
    fn spans_bulk(&self, out: &mut Vec<(u64, u64)>) -> bool {
        let n = self.n;
        out.clear();
        out.resize(n, (0, 0));
        for b in 0..n.div_ceil(BLOCK) {
            let o = &mut out[b * BLOCK..];
            if !self.pos.scan_block(b, |j, s, x| o[j] = (s, x)) {
                return false;
            }
        }
        if !P::GAP_END {
            return true;
        }
        // The ends: the start just past each subtree is in the answer already (the gap waits in the end).
        let o = out.as_mut_ptr();
        for b in 0..n.div_ceil(BLOCK) {
            let sb = self.slot.block(b);
            for j in 0..(n - b * BLOCK).min(BLOCK) {
                let k = b * BLOCK + j;
                let v = sb.get(j);
                let t = v & 1;
                let nx = k + (((v >> 1) & t.wrapping_sub(1)) | t) as usize;
                // SAFETY: k < n, and nx is read only below n.
                unsafe {
                    let z = (*o.add(k)).1;
                    let d = ((z >> 1) as i64) ^ -((z & 1) as i64);
                    // The rightmost path ends with the file: its few ends are read as single ones.
                    (*o.add(k)).1 = if nx < n { ((*o.add(nx)).0 as i64 - d) as u64 } else { self.end(k) };
                }
            }
        }
        true
    }
    fn packed_codes(&self) -> Option<&Packed> {
        Some(&self.code)
    }
    const PREORDER_IDS: bool = true;
}

// ---- B: balanced parentheses ---------------------------------------------------------------------

/// Per byte (bits from the low end): the excess after it, and where the running excess first reaches −k
/// (k = 1..8; 8 = not in this byte). And backwards (bits from the high end): where it first reaches +k.
struct ByteTables {
    total: [i8; 256],
    fwd: [[u8; 9]; 256],
    bwd: [[u8; 9]; 256],
}

static TABLES: ByteTables = byte_tables();

const fn byte_tables() -> ByteTables {
    let mut t = ByteTables { total: [0; 256], fwd: [[8; 9]; 256], bwd: [[8; 9]; 256] };
    let mut b = 0;
    while b < 256 {
        let mut e = 0i32;
        let mut bit = 0;
        while bit < 8 {
            e += if (b >> bit) & 1 == 1 { 1 } else { -1 };
            if e < 0 && t.fwd[b][(-e) as usize] == 8 {
                t.fwd[b][(-e) as usize] = bit as u8;
            }
            bit += 1;
        }
        t.total[b] = e as i8;
        let mut e = 0i32;
        let mut bit = 8;
        while bit > 0 {
            bit -= 1;
            e += if (b >> bit) & 1 == 1 { 1 } else { -1 };
            if e > 0 && t.bwd[b][e as usize] == 8 {
                t.bwd[b][e as usize] = bit as u8;
            }
        }
        b += 1;
    }
    t
}

const L2: usize = 32; // words a second-level summary covers

pub struct B<P: Pos> {
    bits: Vec<u64>,
    nbits: usize,
    /// rank9: ones before each 512-bit superblock, and seven 9-bit counts within it.
    sb_abs: Vec<u32>,
    sb_rel: Vec<u64>,
    /// Per word: the least running excess within it (from its start), and the greatest suffix excess (from
    /// its end backwards); per `L2` words the same.
    w_min: Vec<i8>,
    w_max_suffix: Vec<i8>,
    l2_min: Vec<i16>,
    l2_max_suffix: Vec<i16>,
    l2_total: Vec<i16>,
    code: Packed,
    pos: P,
    /// BH: the opens whose subtree spans more than `BIG` positions, marked in a bit vector over positions
    /// (with rank9), and their sizes, so their close is read, not searched.
    big: Option<Big>,
    /// Preorder number to open position (not part of the tape: only to name the nodes a test asks).
    opens: Vec<u32>,
}

impl<P: Pos> B<P> {
    pub fn new(tree: &Tree, codes: &Codes) -> B<P> {
        B::of(&Cols::of_tree(tree, codes))
    }
    pub fn of(c: &Cols) -> B<P> {
        let n = c.len();
        let mut bits = vec![0u64; (2 * n).div_ceil(64) + 2];
        let mut opens = Vec::with_capacity(n);
        let mut pos = 0usize;
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..n {
            while stack.last().is_some_and(|&e| e <= i) {
                stack.pop();
                pos += 1; // a close
            }
            bits[pos >> 6] |= 1 << (pos & 63);
            opens.push(pos as u32);
            pos += 1;
            stack.push(i + c.size[i] as usize);
        }
        pos += stack.len();
        let nbits = pos;
        let words = nbits.div_ceil(64);
        let (mut sb_abs, mut sb_rel) = (Vec::new(), Vec::new());
        let mut ones = 0u32;
        for s in 0..words.div_ceil(8) {
            sb_abs.push(ones);
            let (mut rel, mut r) = (0u64, 0u64);
            for k in 0..8 {
                let w = s * 8 + k;
                if k > 0 {
                    rel |= r << (9 * (k - 1));
                }
                if w < words {
                    r += u64::from(bits[w].count_ones());
                }
            }
            ones += r as u32;
            sb_rel.push(rel);
        }
        // One more superblock, so a rank just past the last position reads one.
        sb_abs.push(ones);
        sb_rel.push(0);
        let bit = |p: usize| bits[p >> 6] >> (p & 63) & 1 == 1;
        let (mut w_min, mut w_max_suffix) = (Vec::with_capacity(words), Vec::with_capacity(words));
        for w in 0..words {
            let (mut e, mut m) = (0i32, i32::MAX);
            let hi = ((w + 1) * 64).min(nbits);
            for p in w * 64..hi {
                e += if bit(p) { 1 } else { -1 };
                m = m.min(e);
            }
            w_min.push(m.max(-64) as i8);
            let (mut e, mut m) = (0i32, i32::MIN);
            for p in (w * 64..hi).rev() {
                e += if bit(p) { 1 } else { -1 };
                m = m.max(e);
            }
            w_max_suffix.push(m.min(64) as i8);
        }
        let (mut l2_min, mut l2_max_suffix, mut l2_total) = (Vec::new(), Vec::new(), Vec::new());
        for g in 0..words.div_ceil(L2) {
            let (mut e, mut m) = (0i32, i32::MAX);
            let ws: Vec<usize> = (g * L2..((g + 1) * L2).min(words)).collect();
            for &w in &ws {
                m = m.min(e + i32::from(w_min[w]));
                e += word_total(&bits, w, nbits);
            }
            l2_min.push(m as i16);
            l2_total.push(e as i16);
            let (mut e, mut m) = (0i32, i32::MIN);
            for &w in ws.iter().rev() {
                m = m.max(e + i32::from(w_max_suffix[w]));
                e += word_total(&bits, w, nbits);
            }
            l2_max_suffix.push(m as i16);
        }
        B { bits, nbits, sb_abs, sb_rel, w_min, w_max_suffix, l2_min, l2_max_suffix, l2_total, code: code_of(c), pos: P::build(c), big: None, opens }
    }

    /// Ones before position `p`: the preorder number of the open at `p`.
    #[inline(always)]
    fn rank1(&self, p: usize) -> usize {
        let s = p >> 9;
        let k = (p >> 6) & 7;
        // SAFETY: p is a position in the vector, so its superblock and word exist.
        let (abs, rel, word) = unsafe { (*self.sb_abs.get_unchecked(s), *self.sb_rel.get_unchecked(s), *self.bits.get_unchecked(p >> 6)) };
        let within = if k == 0 { 0 } else { (rel >> (9 * (k - 1))) & 511 };
        abs as usize + within as usize + (word & ((1u64 << (p & 63)) - 1)).count_ones() as usize
    }

    #[inline(always)]
    fn is_open(&self, p: usize) -> bool {
        p < self.nbits && self.bits[p >> 6] >> (p & 63) & 1 == 1
    }

    /// The close matching the open at `p`.
    pub fn findclose(&self, p: usize) -> usize {
        let t = &TABLES;
        // Excess needed: the running excess from p + 1 must reach −1.
        let mut need = 1i32;
        let mut q = p + 1;
        // The rest of p's byte, bit by bit (at most 7).
        while q & 7 != 0 {
            need += if self.is_open(q) { 1 } else { -1 };
            if need == 0 {
                return q;
            }
            q += 1;
        }
        loop {
            // Bytes to the end of the word.
            while q & 63 != 0 {
                let byte = (self.bits[q >> 6] >> (q & 63) & 0xff) as usize;
                if need <= 8 && t.fwd[byte][need as usize] != 8 {
                    return q + t.fwd[byte][need as usize] as usize;
                }
                need += i32::from(t.total[byte]);
                q += 8;
            }
            // Whole words, then whole second-level groups.
            let mut w = q >> 6;
            loop {
                if w % L2 == 0 && w / L2 < self.l2_min.len() && i32::from(self.l2_min[w / L2]) > -need {
                    need += i32::from(self.l2_total[w / L2]);
                    w += L2;
                    continue;
                }
                if i32::from(self.w_min[w]) <= -need {
                    break;
                }
                need += word_total(&self.bits, w, self.nbits);
                w += 1;
            }
            q = w * 64;
            let byte = (self.bits[q >> 6] & 0xff) as usize;
            if need <= 8 && t.fwd[byte][need as usize] != 8 {
                return q + t.fwd[byte][need as usize] as usize;
            }
            need += i32::from(t.total[byte]);
            q += 8;
        }
    }

    /// The open enclosing the open at `p` (its parent), if any.
    pub fn enclose(&self, p: usize) -> Option<usize> {
        let t = &TABLES;
        // Going back from p − 1, the running excess must reach +1.
        let mut need = 1i32;
        if p == 0 {
            return None;
        }
        let mut q = p; // the next position to read is q − 1
        while q & 7 != 0 {
            q -= 1;
            need -= if self.is_open(q) { 1 } else { -1 };
            if need == 0 {
                return Some(q);
            }
            if q == 0 {
                return None;
            }
        }
        loop {
            while q & 63 != 0 {
                let byte = (self.bits[(q - 8) >> 6] >> ((q - 8) & 63) & 0xff) as usize;
                if need <= 8 && t.bwd[byte][need as usize] != 8 {
                    return Some(q - 8 + t.bwd[byte][need as usize] as usize);
                }
                need -= i32::from(t.total[byte]);
                q -= 8;
                if q == 0 {
                    return None;
                }
            }
            let mut w = q >> 6; // the words before w remain
            loop {
                if w == 0 {
                    return None;
                }
                if w % L2 == 0 && i32::from(self.l2_max_suffix[w / L2 - 1]) < need {
                    need -= i32::from(self.l2_total[w / L2 - 1]);
                    w -= L2;
                    continue;
                }
                if i32::from(self.w_max_suffix[w - 1]) >= need {
                    break;
                }
                need -= word_total(&self.bits, w - 1, self.nbits);
                w -= 1;
            }
            q = w * 64;
            let byte = (self.bits[(q - 8) >> 6] >> ((q - 8) & 63) & 0xff) as usize;
            if need <= 8 && t.bwd[byte][need as usize] != 8 {
                return Some(q - 8 + t.bwd[byte][need as usize] as usize);
            }
            need -= i32::from(t.total[byte]);
            q -= 8;
        }
    }
}

#[inline(always)]
fn word_total(bits: &[u64], w: usize, nbits: usize) -> i32 {
    let valid = (nbits - w * 64).min(64) as i32;
    2 * bits[w].count_ones() as i32 - valid
}

/// BH's marks: one bit a parentheses position, rank9 over them, and each marked subtree's size.
pub struct Big {
    bits: Vec<u64>,
    sb_abs: Vec<u32>,
    sb_rel: Vec<u64>,
    size: Vec<u32>,
}

/// A subtree spanning more positions than this has its size stored (BH); a smaller one's close is at most
/// this far, so its search reads at most two words.
pub const BIG: usize = 64;

fn rank9(bits: &[u64]) -> (Vec<u32>, Vec<u64>) {
    let words = bits.len();
    let (mut sb_abs, mut sb_rel) = (Vec::new(), Vec::new());
    let mut ones = 0u32;
    for s in 0..words.div_ceil(8) + 1 {
        sb_abs.push(ones);
        let (mut rel, mut r) = (0u64, 0u64);
        for k in 0..8 {
            if k > 0 {
                rel |= r << (9 * (k - 1));
            }
            if let Some(w) = bits.get(s * 8 + k) {
                r += u64::from(w.count_ones());
            }
        }
        ones += r as u32;
        sb_rel.push(rel);
    }
    (sb_abs, sb_rel)
}

impl<P: Pos> B<P> {
    /// BH: B with the sizes of subtrees spanning more than `BIG` positions.
    pub fn with_big(tree: &Tree, codes: &Codes) -> B<P> {
        B::with_big_of(&Cols::of_tree(tree, codes))
    }
    pub fn with_big_of(c: &Cols) -> B<P> {
        let tree = c;
        let mut b = B::of(c);
        let mut bits = vec![0u64; b.nbits.div_ceil(64) + 2];
        let mut size = Vec::new();
        for (i, &s) in tree.size.iter().enumerate() {
            if 2 * s as usize > BIG {
                let p = b.opens[i] as usize;
                bits[p >> 6] |= 1 << (p & 63);
                size.push(s);
            }
        }
        let (sb_abs, sb_rel) = rank9(&bits);
        b.big = Some(Big { bits, sb_abs, sb_rel, size });
        b
    }

    /// The close matching the open at `p`: read for a marked subtree, searched (within `BIG`) otherwise.
    #[inline(always)]
    fn close(&self, p: usize) -> usize {
        if let Some(big) = &self.big {
            let w = big.bits[p >> 6];
            if w >> (p & 63) & 1 == 1 {
                let s = p >> 9;
                let k = (p >> 6) & 7;
                let within = if k == 0 { 0 } else { (big.sb_rel[s] >> (9 * (k - 1))) & 511 };
                let r = big.sb_abs[s] as usize + within as usize + (w & ((1u64 << (p & 63)) - 1)).count_ones() as usize;
                return p + 2 * big.size[r] as usize - 1;
            }
        }
        if self.big.is_some() {
            return self.close_in_window(p);
        }
        self.findclose(p)
    }

    /// The close of an open whose subtree spans at most 64 positions: the 64 bits after it, a byte at a time
    /// through the tables, all eight bytes read and the first hit kept by selects (no branch on the data).
    #[inline(always)]
    fn close_in_window(&self, p: usize) -> usize {
        let t = &TABLES;
        let lo = p + 1;
        let w = extract(&self.bits, lo, 64);
        // Ones before each byte, all eight at once (broadword byte counts, then a prefix by multiplication):
        // byte k starts with need 1 + 2·ones − 8k (the excess still to fall), so the lookups do not wait on
        // each other.
        let mut c = w - ((w >> 1) & 0x5555_5555_5555_5555);
        c = (c & 0x3333_3333_3333_3333) + ((c >> 2) & 0x3333_3333_3333_3333);
        c = (c + (c >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
        let before = c.wrapping_mul(0x0101_0101_0101_0101) << 8;
        let mut found = usize::MAX;
        for k in (0..8).rev() {
            let byte = ((w >> (8 * k)) & 0xff) as usize;
            let ones = ((before >> (8 * k)) & 0xff) as i32;
            let need = 1 + 2 * ones - 8 * k as i32;
            let idx = if (1..=8).contains(&need) { need as usize } else { 0 };
            // SAFETY: a byte and an index of at most 8.
            let f = unsafe { *t.fwd.get_unchecked(byte).get_unchecked(idx) };
            // From the last byte back, so the first hit is the one kept.
            found = if f < 8 { lo + 8 * k + f as usize } else { found };
        }
        found
    }

    /// Bytes by part: code, structure, start, end.
    pub fn parts(&self) -> [usize; 4] {
        let [st, e] = self.pos.bytes();
        let big = self.big.as_ref().map_or(0, |b| b.bits.len() * 8 + b.sb_abs.len() * 4 + b.sb_rel.len() * 8 + b.size.len() * 4);
        let structure = self.nbits.div_ceil(8) + self.sb_abs.len() * 4 + self.sb_rel.len() * 8 + self.w_min.len() * 2 + self.l2_min.len() * 6 + big;
        [self.code.bytes(), structure, st, e]
    }
}

impl<P: Pos> B<P> {
    pub fn image(&self) -> Vec<u8> {
        let mut out = Vec::new();
        raw(&mut out, &self.bits);
        raw(&mut out, &self.sb_abs);
        raw(&mut out, &self.sb_rel);
        raw(&mut out, &self.w_min);
        raw(&mut out, &self.w_max_suffix);
        raw(&mut out, &self.l2_min);
        raw(&mut out, &self.l2_max_suffix);
        raw(&mut out, &self.l2_total);
        if let Some(b) = &self.big {
            raw(&mut out, &b.bits);
            raw(&mut out, &b.sb_abs);
            raw(&mut out, &b.sb_rel);
            raw(&mut out, &b.size);
        }
        self.code.image(&mut out);
        self.pos.image(&mut out);
        out
    }
}

impl<P: Pos> Reader for B<P> {
    fn bytes(&self) -> usize {
        self.parts().iter().sum()
    }
    fn id_of(&self, p: usize) -> usize {
        self.opens[p] as usize
    }
    #[inline(always)]
    fn code(&self, id: usize) -> u32 {
        self.code.get(self.rank1(id)) as u32
    }
    #[inline(always)]
    fn subtree_end(&self, id: usize) -> usize {
        self.close(id) + 1
    }
    #[inline(always)]
    fn parent(&self, id: usize) -> Option<usize> {
        self.enclose(id)
    }
    #[inline(always)]
    fn start(&self, id: usize) -> u64 {
        self.pos.start(self.rank1(id))
    }
    #[inline(always)]
    fn end(&self, id: usize) -> u64 {
        let r = self.rank1(id);
        // Past the subtree: the opens before the position after the close; the close's postorder number:
        // the closes before it.
        let close = std::cell::OnceCell::new();
        let close = || *close.get_or_init(|| self.close(id));
        self.pos.end(r, || self.rank1(close() + 1), || close() - self.rank1(close()))
    }
    #[inline(always)]
    fn first_child(&self, id: usize) -> Option<usize> {
        self.is_open(id + 1).then_some(id + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, id: usize, _pe: usize) -> Option<usize> {
        let j = self.close(id) + 1;
        self.is_open(j).then_some(j)
    }
    #[inline(always)]
    fn next(&self, id: usize) -> Option<usize> {
        // The next open after `id`: the first set bit past it.
        let mut p = id + 1;
        let mut w = p >> 6;
        let mut word = if w < self.bits.len() { self.bits[w] >> (p & 63) } else { 0 };
        loop {
            if word != 0 {
                p += word.trailing_zeros() as usize;
                return (p < self.nbits).then_some(p);
            }
            w += 1;
            if w * 64 >= self.nbits {
                return None;
            }
            p = w * 64;
            word = self.bits[w];
        }
    }
    fn len(&self) -> usize {
        self.opens.len()
    }
    #[inline(always)]
    fn prev(&self, id: usize) -> Option<usize> {
        // The last open before `id`.
        if id == 0 {
            return None;
        }
        let mut w = (id - 1) >> 6;
        let mut word = self.bits[w] & (u64::MAX >> (63 - ((id - 1) & 63)));
        loop {
            if word != 0 {
                return Some(w * 64 + 63 - word.leading_zeros() as usize);
            }
            if w == 0 {
                return None;
            }
            w -= 1;
            word = self.bits[w];
        }
    }
    #[inline(always)]
    fn rank(&self, id: usize) -> usize {
        self.rank1(id)
    }
    #[inline(always)]
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code.get(r) as u32
    }
    /// Starts in order, then one pass over the parentheses: an open pushes its node, a close pops it, and its
    /// end is the start of the next node to open (in the answer already) less its gap. No close is searched.
    fn spans_bulk(&self, out: &mut Vec<(u64, u64)>) -> bool {
        let n = self.len();
        if let Some((st, en)) = self.pos.ef_parts() {
            // Ends in postorder: the k-th close is the k-th end.
            out.clear();
            out.resize(n, (0, 0));
            st.for_each(n, |i, v| out[i].0 = v);
            let mut ends = en.cursor();
            let mut stack: Vec<u32> = Vec::with_capacity(64);
            let mut r = 0u32;
            for (w, &word) in self.bits.iter().enumerate().take(self.nbits.div_ceil(64)) {
                for k in 0..(self.nbits - w * 64).min(64) {
                    if word >> k & 1 == 1 {
                        stack.push(r);
                        r += 1;
                    } else {
                        let q = stack.pop().unwrap();
                        out[q as usize].1 = ends.next();
                    }
                }
            }
            return true;
        }
        if !P::GAP_END {
            return false;
        }
        out.clear();
        out.resize(n, (0, 0));
        {
            let o = out.as_mut_ptr();
            // SAFETY: i < n.
            if !self.pos.scan_all(n, |i, s, z| unsafe { *o.add(i) = (s, z) }) {
                return false;
            }
        }
        let mut stack: Vec<(u32, u32)> = Vec::with_capacity(64);
        let mut r = 0u32;
        for (w, &word) in self.bits.iter().enumerate().take(self.nbits.div_ceil(64)) {
            let m = (self.nbits - w * 64).min(64);
            for k in 0..m {
                if word >> k & 1 == 1 {
                    stack.push((r, (w * 64 + k) as u32));
                    r += 1;
                } else {
                    let (q, p) = stack.pop().unwrap();
                    let z = out[q as usize].1;
                    let d = ((z >> 1) as i64) ^ -((z & 1) as i64);
                    // The rightmost path ends with the file: its few ends are read as single ones.
                    out[q as usize].1 = if (r as usize) < n { (out[r as usize].0 as i64 - d) as u64 } else { self.end(p as usize) };
                }
            }
        }
        true
    }
    #[inline(always)]
    fn end_from_next_start(&self, id: usize, next_start: u64) -> Option<u64> {
        self.pos.end_from_next_start(self.rank1(id), next_start)
    }
    fn packed_codes(&self) -> Option<&Packed> {
        Some(&self.code)
    }
    const PREORDER_IDS: bool = false;
}

// ---- The GPU's view: one buffer, a table of offsets and parameters ----------------------------------

/// A candidate as the GPU reads it: every array in one byte buffer (each at an 8-byte boundary), and a list
/// of parameters (offsets into the buffer, widths, counts) in the order the kernel reads them.
#[derive(Default)]
pub struct GpuImage {
    pub bytes: Vec<u8>,
    pub params: Vec<u32>,
}

impl GpuImage {
    fn arr<T: Copy>(&mut self, v: &[T]) {
        while self.bytes.len() % 8 != 0 {
            self.bytes.push(0);
        }
        self.params.push(self.bytes.len() as u32);
        raw(&mut self.bytes, v);
        // Padding, so a two-word read past the end stays in the buffer.
        self.bytes.extend_from_slice(&[0u8; 16]);
    }
    fn val(&mut self, v: u32) {
        self.params.push(v);
    }
}

pub trait Gpu {
    fn gpu(&self, g: &mut GpuImage);
}

impl Gpu for Packed {
    fn gpu(&self, g: &mut GpuImage) {
        g.val(self.w);
        g.arr(&self.words);
    }
}

impl Gpu for PFor {
    fn gpu(&self, g: &mut GpuImage) {
        g.val(u32::from(self.with_base));
        g.val(self.n as u32);
        g.arr(&self.head);
        if self.with_base { g.arr(&self.base) } else { g.val(0) }
        g.arr(&self.words);
    }
}

impl Gpu for Pred {
    fn gpu(&self, g: &mut GpuImage) {
        g.arr(&self.slope);
        g.arr(&self.base);
        self.res.gpu(g);
    }
}

impl Gpu for Ef {
    fn gpu(&self, g: &mut GpuImage) {
        g.val(self.l);
        self.low.gpu(g);
        g.arr(&self.high);
        g.arr(&self.samples);
    }
}

impl Gpu for ForLen {
    fn gpu(&self, g: &mut GpuImage) {
        self.start.gpu(g);
        self.len.gpu(g);
    }
}

impl Gpu for PredGap {
    fn gpu(&self, g: &mut GpuImage) {
        self.start.gpu(g);
        self.gap.gpu(g);
        g.val(self.start.n as u32);
        g.val(self.source_len as u32);
        g.val((self.source_len >> 32) as u32);
    }
}

impl Gpu for EfEf {
    fn gpu(&self, g: &mut GpuImage) {
        self.start.gpu(g);
        self.end.gpu(g);
    }
}

impl<P: Pos + Gpu> X<P> {
    pub fn gpu_image(&self) -> GpuImage {
        let mut g = GpuImage::default();
        g.val(self.n as u32);
        self.code.gpu(&mut g);
        self.size.gpu(&mut g);
        self.back.gpu(&mut g);
        self.pos.gpu(&mut g);
        g
    }
}

impl<P: Pos + Gpu> XF<P> {
    pub fn gpu_image(&self) -> GpuImage {
        let mut g = GpuImage::default();
        g.val(self.n as u32);
        self.code.gpu(&mut g);
        self.slot.gpu(&mut g);
        self.pos.gpu(&mut g);
        g
    }
}

impl<P: Pos + Gpu> B<P> {
    pub fn gpu_image(&self) -> GpuImage {
        let mut g = GpuImage::default();
        g.val(self.nbits as u32);
        g.arr(&self.bits);
        g.arr(&self.sb_abs);
        g.arr(&self.sb_rel);
        g.arr(&self.w_min);
        g.arr(&self.w_max_suffix);
        g.arr(&self.l2_min);
        g.arr(&self.l2_max_suffix);
        g.arr(&self.l2_total);
        g.val(self.l2_min.len() as u32);
        g.val(self.opens.len() as u32);
        self.code.gpu(&mut g);
        self.pos.gpu(&mut g);
        g
    }
}

/// The byte tables, for the GPU: `total` (256 i8), `fwd` and `bwd` (256 × 9 u8 each).
pub fn byte_tables_bytes() -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 19);
    out.extend(TABLES.total.iter().map(|&v| v as u8));
    for r in &TABLES.fwd {
        out.extend_from_slice(r);
    }
    for r in &TABLES.bwd {
        out.extend_from_slice(r);
    }
    out
}

/// All of a corpus's trees as one forest, under a root of kind 0, each file's positions after the last's:
/// one tape a GPU dispatch can ask about.
pub fn forest(trees: &[Tree]) -> Tree {
    use crate::tree::Node;
    let total: usize = trees.iter().map(Tree::len).sum::<usize>() + 1;
    let mut nodes = Vec::with_capacity(total);
    let mut size = Vec::with_capacity(total);
    let mut parent = Vec::with_capacity(total);
    let src: u64 = trees.iter().map(|t| t.source_len).sum();
    nodes.push(Node { kind: 0, field: 0, flags: 0, classes: vec![], start: 0, end: src });
    size.push(total as u32);
    parent.push(NONE);
    let mut shift = 0u64;
    for t in trees {
        let base = nodes.len() as u32;
        for (i, n) in t.nodes.iter().enumerate() {
            nodes.push(Node { start: n.start + shift, end: n.end + shift, ..n.clone() });
            size.push(t.size[i]);
            parent.push(if t.parent[i] == NONE { 0 } else { t.parent[i] + base });
        }
        shift += t.source_len;
    }
    Tree { nodes, size, parent, source_len: src }
}

/// The select table, for the GPU (256 × 8).
pub fn select8_bytes() -> Vec<u8> {
    SELECT8.iter().flatten().copied().collect()
}

/// Bits to hold `vals` under each packing considered for XF's slots (E11): PFOR at blocks of 64, 128, 256
/// (exceptions 40 bits, header 40 bits); escape-indexed exceptions (the top codes of a block's width name an
/// exception, packed at the block's exception width; header 64 bits) at 64, 128, 256; and a block bitmap
/// with a rank (w1 + 1 bit a value, exceptions at their width, header 64 bits) at 256.
pub fn pack_costs(vals: &[u64]) -> Vec<u64> {
    let mut out = Vec::new();
    for blk in [64usize, 128, 256] {
        let mut t = 0u64;
        for c in vals.chunks(blk) {
            let best = (0..=32u32).map(|w| {
                let e = c.iter().filter(|&&v| v >= mask(w)).count() as u64;
                if w == 0 && e > 0 { u64::MAX } else { c.len() as u64 * w as u64 + if w == 0 { 0 } else { e * 40 } }
            }).min().unwrap();
            t += best + 40;
        }
        out.push(t);
    }
    for blk in [64usize, 128, 256] {
        let mut t = 0u64;
        for c in vals.chunks(blk) {
            let mut best = u64::MAX;
            for w in 0..=32u32 {
                let lim = |e: u64| (1u64 << w).saturating_sub(e);
                // E exceptions take the top E codes; a value is literal below 2^w - E.
                let mut e = 0u64;
                loop {
                    let ne = c.iter().filter(|&&v| v >= lim(e)).count() as u64;
                    if ne <= e { break; }
                    e = ne;
                    if e > (1u64 << w) { break; }
                }
                if e > (1u64 << w) || (e > 0 && lim(e) == 0 && w == 0) { continue; }
                let ew = c.iter().filter(|&&v| v >= lim(e)).map(|&v| bits_for(v)).max().unwrap_or(0) as u64;
                best = best.min(c.len() as u64 * w as u64 + e * ew);
            }
            t += best + 64;
        }
        out.push(t);
    }
    let mut t = 0u64;
    for c in vals.chunks(256) {
        let best = (0..=32u32).map(|w| {
            let big: Vec<u64> = c.iter().copied().filter(|&v| v > mask(w)).collect();
            let ew = big.iter().map(|&v| bits_for(v)).max().unwrap_or(0) as u64;
            c.len() as u64 * (w as u64 + 1) + big.len() as u64 * ew
        }).min().unwrap();
        t += best + 64;
    }
    out.push(t);
    out.push(vals.len() as u64);
    out
}

// ---- XR: XF's fields as one record a node ---------------------------------------------------------

/// XF with a node's fields side by side: each block of `BLOCK` nodes is a run of records (code, slot,
/// start above the block's least, length), each field at the block's own width, then each field's
/// exceptions. A field's top codes are escapes naming its exception, as in `PFor`. So a node's fields
/// share a cache line, where XF's four arrays touch four.
///
/// A block's header is three words: the bit offset and the four widths; each escaping field's escape count
/// and exception width; the block's least start.
pub struct XR {
    n: usize,
    head: Vec<[u64; 3]>,
    words: Vec<u64>,
}

impl XR {
    pub fn new(tree: &Tree, codes: &Codes) -> XR {
        XR::of(&Cols::of_tree(tree, codes))
    }
    pub fn of(c: &Cols) -> XR {
        let n = c.len();
        let slot: Vec<u64> = (0..n).map(|i| {
            if c.size[i] > 1 {
                u64::from(c.size[i]) << 1
            } else if i + 1 < n {
                (((i + 1 - c.parent[i + 1] as usize) as u64) << 1) | 1
            } else {
                1
            }
        }).collect();
        let len: Vec<u64> = c.start.iter().zip(&c.end).map(|(s, e)| e - s).collect();
        let mut x = XR { n, head: Vec::new(), words: vec![0; 2] };
        let mut bit = 0usize;
        for b in 0..n.div_ceil(BLOCK) {
            let r = b * BLOCK..((b + 1) * BLOCK).min(n);
            let m = r.len();
            let base = c.start[r.clone()].iter().copied().min().unwrap();
            let wc = c.code[r.clone()].iter().map(|&v| bits_for(v)).max().unwrap();
            let fields: [Vec<u64>; 3] = [slot[r.clone()].to_vec(), c.start[r.clone()].iter().map(|&s| s - base).collect(), len[r.clone()].to_vec()];
            let shape: Vec<(u32, u32, u32)> = fields.iter().map(|f| esc_width(f)).collect();
            let w = [wc, shape[0].0, shape[1].0, shape[2].0];
            let rec: usize = w.iter().map(|&v| v as usize).sum();
            assert!(w.iter().all(|&v| v < 64) && (bit as u64) < 1 << 40);
            let h0 = bit as u64 | u64::from(w[0]) << 40 | u64::from(w[1]) << 46 | u64::from(w[2]) << 52 | u64::from(w[3]) << 58;
            let mut h1 = 0u64;
            for (k, &(_, e, ew)) in shape.iter().enumerate() {
                h1 |= (u64::from(e) | u64::from(ew) << 9) << (15 * k);
            }
            x.head.push([h0, h1, base]);
            let exc_total: usize = shape.iter().map(|&(_, e, ew)| e as usize * ew as usize).sum();
            let end = bit + m * rec + exc_total;
            x.words.resize(end / 64 + 2, 0);
            // The exceptions' areas, in field order, after the records.
            let mut exc_at = bit + m * rec;
            let mut kth = [0u64; 3];
            let mut areas = [0usize; 3];
            for (k, &(_, e, ew)) in shape.iter().enumerate() {
                areas[k] = exc_at;
                exc_at += e as usize * ew as usize;
            }
            for j in 0..m {
                let mut at = bit + j * rec;
                or_in(&mut x.words, at, w[0], c.code[r.start + j]);
                at += w[0] as usize;
                for (k, f) in fields.iter().enumerate() {
                    let (wk, e, ew) = shape[k];
                    let lim = (1u64 << wk) - u64::from(e);
                    let v = f[j];
                    if v >= lim {
                        or_in(&mut x.words, at, wk, lim + kth[k]);
                        or_in(&mut x.words, areas[k] + kth[k] as usize * ew as usize, ew, v);
                        kth[k] += 1;
                    } else {
                        or_in(&mut x.words, at, wk, v);
                    }
                    at += wk as usize;
                }
            }
            bit = end;
        }
        x
    }
    pub fn parts(&self) -> [usize; 4] {
        // One array: counted as structure, all of it (the parts are not apart).
        [0, self.head.len() * 24 + self.words.len() * 8 + 8, 0, 0]
    }
    pub fn image(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for h in &self.head {
            raw(&mut out, h);
        }
        raw(&mut out, &self.words);
        out
    }
    /// Field `k` of node i (0 code, 1 slot, 2 start, 3 length).
    #[inline(always)]
    fn field(&self, i: usize, k: usize) -> u64 {
        let b = i / BLOCK;
        // SAFETY: i is a node of the tape.
        let h = unsafe { self.head.get_unchecked(b) };
        let off = (h[0] & ((1 << 40) - 1)) as usize;
        let w = [(h[0] >> 40) & 63, (h[0] >> 46) & 63, (h[0] >> 52) & 63, (h[0] >> 58) & 63];
        let rec = (w[0] + w[1] + w[2] + w[3]) as usize;
        let j = i % BLOCK;
        let mut at = off + j * rec;
        for &wk in &w[..k] {
            at += wk as usize;
        }
        let wk = w[k] as u32;
        let v = extract(&self.words, at, wk);
        let v = if k == 0 {
            v
        } else {
            let e = (h[1] >> (15 * (k - 1))) & 511;
            let lim = (1u64 << wk) - e;
            if v >= lim {
                let m = (self.n - b * BLOCK).min(BLOCK);
                let mut area = off + m * rec;
                for q in 0..k - 1 {
                    let s = h[1] >> (15 * q);
                    area += ((s & 511) * ((s >> 9) & 63)) as usize;
                }
                let ew = ((h[1] >> (15 * (k - 1) + 9)) & 63) as u32;
                extract(&self.words, area + (v - lim) as usize * ew as usize, ew)
            } else {
                v
            }
        };
        if k == 2 { v + h[2] } else { v }
    }
}

impl Reader for XR {
    fn bytes(&self) -> usize {
        self.parts().iter().sum()
    }
    fn id_of(&self, p: usize) -> usize {
        p
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        self.field(i, 0) as u32
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        let v = self.field(i, 1);
        let t = v & 1;
        i + (((v >> 1) & t.wrapping_sub(1)) | t) as usize
    }
    #[inline(always)]
    fn parent(&self, i: usize) -> Option<usize> {
        if i == 0 {
            return None;
        }
        let v = self.field(i - 1, 1);
        let t = v & 1;
        Some(i - (((v >> 1) & t.wrapping_neg()) | (t ^ 1)) as usize)
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        self.field(i, 2)
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        self.field(i, 2) + self.field(i, 3)
    }
    #[inline(always)]
    fn first_child(&self, i: usize) -> Option<usize> {
        (self.field(i, 1) & 1 == 0).then_some(i + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, i: usize, pe: usize) -> Option<usize> {
        let j = self.subtree_end(i);
        (j < pe && j < self.n).then_some(j)
    }
    #[inline(always)]
    fn next(&self, id: usize) -> Option<usize> {
        (id + 1 < self.n).then_some(id + 1)
    }
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn prev(&self, id: usize) -> Option<usize> {
        id.checked_sub(1)
    }
    #[inline(always)]
    fn rank(&self, id: usize) -> usize {
        id
    }
    #[inline(always)]
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code(r)
    }
    const PREORDER_IDS: bool = true;
}
