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

impl Packed {
    pub fn new(vals: &[u64]) -> Packed {
        let w = vals.iter().map(|&v| bits_for(v)).max().unwrap_or(0);
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

pub struct X<'c> {
    n: usize,
    code: Packed,
    size: PFor,
    back: PFor,
    start: PFor,
    len: PFor,
    _codes: std::marker::PhantomData<&'c Codes>,
}

/// The arrays `X` and `B` share, by preorder number.
pub struct Fields {
    pub code: Packed,
    pub start: PFor,
    pub len: PFor,
}

pub fn fields(tree: &Tree, codes: &Codes) -> Fields {
    let code: Vec<u64> = tree.nodes.iter().map(|n| u64::from(codes.ids[&(n.kind, n.field, n.flags, n.classes.clone())])).collect();
    let start: Vec<u64> = tree.nodes.iter().map(|n| n.start).collect();
    let len: Vec<u64> = tree.nodes.iter().map(|n| n.end - n.start).collect();
    Fields { code: Packed::new(&code), start: PFor::new(&start, true), len: PFor::new(&len, false) }
}

impl<'c> X<'c> {
    pub fn new(tree: &Tree, codes: &'c Codes) -> X<'c> {
        let f = fields(tree, codes);
        let size: Vec<u64> = tree.size.iter().map(|&s| u64::from(s)).collect();
        let back: Vec<u64> = tree.parent.iter().enumerate().map(|(i, &p)| if p == NONE { 0 } else { (i - p as usize) as u64 }).collect();
        X { n: tree.len(), code: f.code, size: PFor::new(&size, false), back: PFor::new(&back, false), start: f.start, len: f.len, _codes: std::marker::PhantomData }
    }
}

impl X<'_> {
    /// Bytes by part.
    pub fn parts(&self) -> [(&'static str, usize); 5] {
        [("code", self.code.bytes()), ("size", self.size.bytes()), ("back", self.back.bytes()), ("start", self.start.bytes()), ("length", self.len.bytes())]
    }
}

impl Reader for X<'_> {
    fn bytes(&self) -> usize {
        self.code.bytes() + self.size.bytes() + self.back.bytes() + self.start.bytes() + self.len.bytes()
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
        self.start.get(i)
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        self.start.get(i) + self.len.get(i)
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

static TABLES: std::sync::LazyLock<ByteTables> = std::sync::LazyLock::new(|| {
    let mut t = ByteTables { total: [0; 256], fwd: [[8; 9]; 256], bwd: [[8; 9]; 256] };
    for b in 0..256usize {
        let mut e = 0i32;
        for bit in 0..8 {
            e += if b >> bit & 1 == 1 { 1 } else { -1 };
            if e < 0 && t.fwd[b][(-e) as usize] == 8 {
                t.fwd[b][(-e) as usize] = bit as u8;
            }
        }
        t.total[b] = e as i8;
        let mut e = 0i32;
        for bit in (0..8).rev() {
            e += if b >> bit & 1 == 1 { 1 } else { -1 };
            if e > 0 && t.bwd[b][e as usize] == 8 {
                t.bwd[b][e as usize] = bit as u8;
            }
        }
    }
    t
});

const L2: usize = 32; // words a second-level summary covers

pub struct B {
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
    f: Fields,
    /// Preorder number to open position (not part of the tape: only to name the nodes a test asks).
    opens: Vec<u32>,
}

impl B {
    pub fn new(tree: &Tree, codes: &Codes) -> B {
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
        B { bits, nbits, sb_abs, sb_rel, w_min, w_max_suffix, l2_min, l2_max_suffix, l2_total, f: fields(tree, codes), opens }
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
        let t = &*TABLES;
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
        let t = &*TABLES;
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

impl B {
    /// Bytes by part.
    pub fn parts(&self) -> [(&'static str, usize); 5] {
        [
            ("parentheses", self.nbits.div_ceil(8)),
            ("rank + excess summaries", self.sb_abs.len() * 4 + self.sb_rel.len() * 8 + self.w_min.len() * 2 + self.l2_min.len() * 6),
            ("code", self.f.code.bytes()),
            ("start", self.f.start.bytes()),
            ("length", self.f.len.bytes()),
        ]
    }
}

impl Reader for B {
    fn bytes(&self) -> usize {
        self.nbits.div_ceil(8) + self.sb_abs.len() * 4 + self.sb_rel.len() * 8 + self.w_min.len() * 2 + self.l2_min.len() * 6
            + self.f.code.bytes() + self.f.start.bytes() + self.f.len.bytes()
    }
    fn id_of(&self, p: usize) -> usize {
        self.opens[p] as usize
    }
    #[inline(always)]
    fn code(&self, id: usize) -> u32 {
        self.f.code.get(self.rank1(id)) as u32
    }
    #[inline(always)]
    fn subtree_end(&self, id: usize) -> usize {
        self.findclose(id) + 1
    }
    #[inline(always)]
    fn parent(&self, id: usize) -> Option<usize> {
        self.enclose(id)
    }
    #[inline(always)]
    fn start(&self, id: usize) -> u64 {
        self.f.start.get(self.rank1(id))
    }
    #[inline(always)]
    fn end(&self, id: usize) -> u64 {
        let r = self.rank1(id);
        self.f.start.get(r) + self.f.len.get(r)
    }
    #[inline(always)]
    fn first_child(&self, id: usize) -> Option<usize> {
        self.is_open(id + 1).then_some(id + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, id: usize, _pe: usize) -> Option<usize> {
        let j = self.findclose(id) + 1;
        self.is_open(j).then_some(j)
    }
}
