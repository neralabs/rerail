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

fn raw<T: Copy>(out: &mut Vec<u8>, v: &[T]) {
    // SAFETY: plain integers, read as bytes.
    out.extend_from_slice(unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) });
}

#[inline(always)]
fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

/// `w` bits at bit `bit` of `words` (one padding word at the end).
#[inline(always)]
fn extract(words: &[u64], bit: usize, w: u32) -> u64 {
    let k = bit >> 6;
    let o = (bit & 63) as u32;
    // SAFETY: every writer pads one word past the last value, so k + 1 is in bounds.
    let (lo, hi) = unsafe { (*words.get_unchecked(k), *words.get_unchecked(k + 1)) };
    // `(hi << 1) << (63 - o)` is `hi << (64 - o)` without the shift by 64 when o is 0.
    ((lo >> o) | ((hi << 1) << (63 - o))) & mask(w)
}

fn put(words: &mut Vec<u64>, bit: usize, w: u32, v: u64) {
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

fn bits_for(v: u64) -> u32 {
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
}

/// A block of `BLOCK` values at a time: a base (the block's least value), a width, and the values above
/// the base; a value that does not fit is all ones, and its value is in the block's exceptions.
pub struct PFor {
    width: Vec<u8>,
    bitoff: Vec<u32>,
    base: Vec<u64>,
    words: Vec<u64>,
    exc_at: Vec<u32>,
    exc_idx: Vec<u8>,
    exc_val: Vec<u32>,
    with_base: bool,
}

impl Image for PFor {
    fn image(&self, out: &mut Vec<u8>) {
        raw(out, &self.width);
        raw(out, &self.bitoff);
        if self.with_base {
            raw(out, &self.base);
        }
        raw(out, &self.words);
        raw(out, &self.exc_at);
        raw(out, &self.exc_idx);
        raw(out, &self.exc_val);
    }
}

impl PFor {
    pub fn new(vals: &[u64], with_base: bool) -> PFor {
        let mut p = PFor { width: vec![], bitoff: vec![], base: vec![], words: vec![0; 2], exc_at: vec![0], exc_idx: vec![], exc_val: vec![], with_base };
        let mut bit = 0usize;
        for chunk in vals.chunks(BLOCK) {
            let b = if with_base { *chunk.iter().min().unwrap() } else { 0 };
            let rel: Vec<u64> = chunk.iter().map(|&v| v - b).collect();
            // The width that costs least: values, plus 5 bytes an exception.
            let mut best = (u64::MAX, 0u32);
            for w in 0..=32u32 {
                let lim = mask(w);
                let exc = rel.iter().filter(|&&v| v >= lim).count() as u64;
                // A width of 0 holds only zeros and has no escape.
                if w == 0 && exc > 0 && rel.iter().any(|&v| v != 0) {
                    continue;
                }
                let exc = if w == 0 { 0 } else { exc };
                let cost = chunk.len() as u64 * w as u64 + exc * 40;
                if cost < best.0 {
                    best = (cost, w);
                }
            }
            let w = best.1;
            p.width.push(w as u8);
            p.bitoff.push(bit as u32);
            p.base.push(b);
            for (j, &v) in rel.iter().enumerate() {
                if w > 0 && v >= mask(w) {
                    put(&mut p.words, bit, w, mask(w));
                    p.exc_idx.push(j as u8);
                    p.exc_val.push(u32::try_from(v).expect("an exception within 32 bits"));
                } else {
                    put(&mut p.words, bit, w, v);
                }
                bit += w as usize;
            }
            p.exc_at.push(p.exc_idx.len() as u32);
        }
        put(&mut p.words, bit, 0, 0);
        p
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        let b = i / BLOCK;
        // SAFETY: i is within the array the reader was built over.
        let (w, off) = unsafe { (*self.width.get_unchecked(b) as u32, *self.bitoff.get_unchecked(b) as usize) };
        let v = extract(&self.words, off + (i % BLOCK) * w as usize, w);
        let base = if self.with_base { unsafe { *self.base.get_unchecked(b) } } else { 0 };
        if w > 0 && v == mask(w) {
            let (a, z) = (self.exc_at[b] as usize, self.exc_at[b + 1] as usize);
            let j = (i % BLOCK) as u8;
            let k = a + self.exc_idx[a..z].iter().position(|&x| x == j).unwrap();
            return base + u64::from(self.exc_val[k]);
        }
        base + v
    }
    pub fn bytes(&self) -> usize {
        self.width.len() * 5 + if self.with_base { self.base.len() * 8 } else { 0 } + self.words.len() * 8 + self.exc_at.len() * 4 + self.exc_idx.len() * 5
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
}

// ---- X: explicit, bit-packed ---------------------------------------------------------------------

/// Each node's code at the language's width.
pub fn code_of(tree: &Tree, codes: &Codes) -> Packed {
    let code: Vec<u64> = tree.nodes.iter().map(|n| u64::from(codes.ids[&(n.kind, n.field, n.flags, n.classes.clone())])).collect();
    Packed::new(&code)
}

/// How a candidate stores start and end, by preorder number `r`. `next` gives the preorder number just past
/// the node's subtree (n past the last), `post` its postorder number; each is asked only if the form needs it.
pub trait Pos: Sized + Image {
    const NAME: &'static str;
    fn build(tree: &Tree) -> Self;
    fn bytes(&self) -> [usize; 2];
    fn start(&self, r: usize) -> u64;
    fn end(&self, r: usize, next: impl FnOnce() -> usize, post: impl FnOnce() -> usize) -> u64;
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
    fn build(tree: &Tree) -> Self {
        let start: Vec<u64> = tree.nodes.iter().map(|n| n.start).collect();
        let len: Vec<u64> = tree.nodes.iter().map(|n| n.end - n.start).collect();
        ForLen { start: PFor::new(&start, true), len: PFor::new(&len, false) }
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
    fn build(tree: &Tree) -> Self {
        let n = tree.len();
        let start: Vec<u64> = tree.nodes.iter().map(|x| x.start).collect();
        let gap: Vec<u64> = (0..n).map(|i| {
            let next = i + tree.size[i] as usize;
            let s = if next < n { start[next] } else { tree.source_len };
            let d = s as i64 - tree.nodes[i].end as i64;
            ((d << 1) ^ (d >> 63)) as u64
        }).collect();
        PredGap { start: Pred::new(&start), gap: PFor::new(&gap, false), source_len: tree.source_len }
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
    fn build(tree: &Tree) -> Self {
        let start: Vec<u64> = tree.nodes.iter().map(|n| n.start).collect();
        let len: Vec<u64> = tree.nodes.iter().map(|n| n.end - n.start).collect();
        PredLen { start: Pred::new(&start), len: PFor::new(&len, false) }
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
    fn build(tree: &Tree) -> Self {
        let g = PredGap::build(tree);
        let start: Vec<u64> = tree.nodes.iter().map(|x| x.start).collect();
        EfGap { start: Ef::new(&start, tree.source_len), n: tree.len(), gap: g.gap, source_len: tree.source_len }
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
    fn build(tree: &Tree) -> Self {
        let n = tree.len();
        let start: Vec<u64> = tree.nodes.iter().map(|x| x.start).collect();
        // Postorder: a node after its subtree.
        let mut post = Vec::with_capacity(n);
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..n {
            while let Some(&top) = stack.last() {
                if top + tree.size[top] as usize <= i {
                    post.push(tree.nodes[top].end);
                    stack.pop();
                } else {
                    break;
                }
            }
            stack.push(i);
        }
        while let Some(top) = stack.pop() {
            post.push(tree.nodes[top].end);
        }
        EfEf { start: Ef::new(&start, tree.source_len), end: Ef::new(&post, tree.source_len) }
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
        for chunk in vals.chunks(BLOCK) {
            let fit = |sl: u64| -> (u64, Vec<u64>) {
                let r: Vec<i64> = chunk.iter().enumerate().map(|(j, &v)| v as i64 - ((j as u64 * sl) >> 16) as i64).collect();
                let lo = *r.iter().min().unwrap();
                (lo as u64, r.iter().map(|&x| (x - lo) as u64).collect())
            };
            let span = chunk.last().unwrap() - chunk[0];
            let line = if chunk.len() > 1 { (span << 16) / (chunk.len() as u64 - 1) } else { 0 };
            let cost = |r: &[u64]| -> u64 { r.iter().map(|&x| u64::from(bits_for(x))).max().unwrap_or(0) };
            let (b0, r0) = fit(0);
            let (b1, r1) = fit(line.min(u64::from(u32::MAX)));
            let (sl, b, r) = if cost(&r1) < cost(&r0) { (line.min(u64::from(u32::MAX)), b1, r1) } else { (0, b0, r0) };
            slope.push(sl as u32);
            base.push(b);
            res.extend(r);
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
        let size: Vec<u64> = tree.size.iter().map(|&s| u64::from(s)).collect();
        let back: Vec<u64> = tree.parent.iter().enumerate().map(|(i, &p)| if p == NONE { 0 } else { (i - p as usize) as u64 }).collect();
        X { n: tree.len(), code: code_of(tree, codes), size: PFor::new(&size, false), back: PFor::new(&back, false), pos: P::build(tree) }
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
        let n = tree.len();
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
            stack.push(i + tree.size[i] as usize);
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
        B { bits, nbits, sb_abs, sb_rel, w_min, w_max_suffix, l2_min, l2_max_suffix, l2_total, code: code_of(tree, codes), pos: P::build(tree), big: None, opens }
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
        let mut b = B::new(tree, codes);
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
        g.arr(&self.width);
        g.arr(&self.bitoff);
        if self.with_base { g.arr(&self.base) } else { g.val(0) }
        g.arr(&self.words);
        g.arr(&self.exc_at);
        g.arr(&self.exc_idx);
        g.arr(&self.exc_val);
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
