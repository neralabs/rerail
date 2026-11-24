//! E24 (t39 step 16): format 16's in-memory tape as one form, read in place (0156 sections 1–2).
//!
//! The parts each experiment chose, together, every array at a whole-byte width fixed a language:
//!
//! - **codes**: one byte a node, numbered by frequency a language (E21, `code8::Rank8`): a byte below the
//!   cutoff is the code's number; past it, a prefix and the block's side list;
//! - **slots**: the leaf slot by sibling (E16), holding the parent distance on every 8th sibling (and wherever
//!   it is at most `FIT`, 0 now: t39 step 23): an inner node `size << 1`, a leaf `parent distance << 2 | 3` or
//!   `size(previous sibling of the next node) << 2 | 1`;
//! - **positions**: each start as its gap to the previous start, each end as a length, and the absolute
//!   start of every 8th node in a checkpoint array (E16);
//! - **escapes**: a value too wide for its array's width is the all-ones value, listed in its 256-node block's
//!   side list (`repeat::Esc`).
//!
//! A language's numbering and widths are learned once (`F16Lang::learn`) and fixed, so a writer knows them
//! before a file starts, and a repeated subtree is the same bytes in every array but the checkpoints.

use crate::code8::{Rank8, numbers_by};
use crate::inplace::{Cols, Codes, Packed, Reader, bits_for};
use crate::relative::{gaps, sib_slots, sib_slots_fit};
use crate::repeat::{Esc, Hist};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
const HI: u64 = 0x8080_8080_8080_8080;

/// Every 8th sibling holds its parent distance; every 8th node's start is a checkpoint.
pub const K: usize = 8;
const KS: u32 = K.trailing_zeros();

/// A set of codes as the reader tests it: by number for the bytes below the cutoff, and whether any code
/// past it is in the set (those bytes are then decoded).
pub struct NumSet {
    pub byte: [bool; 256],
    pub tail: bool,
    /// By number, every code (compiled once, with the set), when `tail`.
    pub by_num: Vec<bool>,
}

/// A language's fixed choices: the code numbering, the cutoff, and the widths of slots, gaps and lengths.
pub struct F16Lang {
    /// Code (the language's `Codes` index) to number, and back.
    pub num: Vec<u32>,
    pub by_num: Vec<u32>,
    pub cut: u32,
    /// The code table's size, from the grammar (`grammar_codes`).
    pub table: usize,
    /// The revision its tapes are marked with (`revision`).
    pub version: u8,
    /// Slots, gaps, lengths, in bits (multiples of 8).
    pub widths: [u32; 3],
    /// Sets compiled to numbers, once per set (a rule's set is compiled once, not per file).
    sets: RefCell<HashMap<(usize, usize), Rc<NumSet>>>,
}

/// The codes a language's grammar allows (E25, `tape16 gcodes`, which derives them from node-types.json and
/// checks every captured node against them): the table every tape of the language numbers into. `.rerail` and
/// Blade, whose parsers declare no node types, are the bound kinds × fields × flags.
pub fn grammar_codes(tag: u8) -> usize {
    match tag {
        1 => 2045,
        2 => 3647,
        3 => 3696,
        6 => 2419,
        7 => 2761,
        8 => 2035,
        9 => 1843,
        10 => 1937,
        12 => 76,
        13 => 14896,
        14 => 70,
        _ => 0,
    }
}

/// The tape's version mark (0156 §1.0): byte 0 of every tape, in memory and at rest, the language's tape
/// revision. One number covers every choice a reader must share with the writer: the layout, the code table
/// and its numbering, the widths, K and N, the dictionaries. 1–254 are revisions; 0 is never written (zeroed
/// bytes are not a tape); 255 says the revision goes on in the next two bytes.
/// The line terminators of tag `tag`'s language (decision 0612), as the engine's adapters name them: the tape's
/// line index's.
pub fn terminators(tag: u8) -> rerail_tape_format::newlines::Terminators {
    use rerail_tape_format::newlines::Terminators;
    match tag {
        2 | 3 | 6 | 12 | 31 => Terminators::Eslint,
        9 | 27 => Terminators::Lf,
        19 => Terminators::Roslyn,
        21 => Terminators::Css,
        _ => Terminators::Newlines,
    }
}

/// Every table's revision: 5 since the grammars' tables are numbered by 520 repositories' HEAD trees and the rule
/// files (decision 0638); 4 added the spike's counts, 3 was the spike's alone; 2 the grammar's order (decision 0631);
/// 1 was ordered by captures and bounded by them.
pub fn revision(tag: u8) -> u8 {
    let _ = tag;
    5
}
pub const MARK_LONG: u8 = 255;

/// The largest parent distance a leaf slot names off the K-th sibling (t39 step 23); 0 is E16's rule, the
/// parent on every K-th sibling only. Measured: 15 takes 73–75% of the sibling steps away for up to 1.6% more
/// on disk (every distance that fits the width, 85% for up to 2.8%), but only point parents read them, 10–20%
/// faster; the bulk primitives read no parent (prefix max) or few (the bounded climb), so 0 is kept.
pub const FIT: u64 = 0;

/// The slot values below which a leaf names its parent, at slot width `w` (0: none, the E16 rule).
pub fn fit_limit(w: u32) -> u64 {
    if FIT == 0 { 0 } else { ((FIT << 2) | 4).min((1u64 << w) - 1) }
}

/// The checkpoints' width for a file of `source_len` bytes: whole bytes.
pub fn ckpt_width(source_len: u64) -> u32 {
    bits_for(source_len).max(1).next_multiple_of(8)
}

/// A language's statistics, gathered file by file (nothing held), then fixed by `finish`.
#[derive(Default)]
pub struct Learner {
    freq: Vec<u64>,
    hs: Hist,
    hg: Hist,
    hl: Hist,
}

impl Learner {
    pub fn add(&mut self, c: &Cols) {
        for &k in &c.code {
            if self.freq.len() <= k as usize {
                self.freq.resize(k as usize + 1, 0);
            }
            self.freq[k as usize] += 1;
        }
        self.hs.add(sib_slots(c, K).into_iter());
        let (g, l) = gaps(c);
        self.hg.add(g.into_iter());
        self.hl.add(l.into_iter());
    }
    /// `table`: the language's code table from the grammar (`grammar_codes`), at least the capture's codes.
    pub fn finish(mut self, codes: &Codes, table: usize) -> F16Lang {
        self.freq.resize(codes.table.len(), 0);
        let table = table.max(codes.table.len());
        let num = numbers_by(&self.freq);
        let mut by_num = vec![0u32; num.len()];
        for (k, &n) in num.iter().enumerate() {
            by_num[n as usize] = k as u32;
        }
        // One prefix, 255, when the grammar's table passes 256 codes: the low part past it is one byte to 511
        // codes, two beyond (E25). The codes the corpus never showed are numbered after those it did.
        let cut = if table <= 256 { 256 } else { 255 };
        F16Lang { version: 0, num, by_num, cut, table, widths: [self.hs.width_bytes(), self.hg.width_bytes(), self.hl.width_bytes()], sets: RefCell::default() }
    }
}

impl F16Lang {
    /// The same choices, marked with the language's revision.
    pub fn marked(mut self, tag: u8) -> F16Lang {
        self.version = revision(tag);
        self
    }
    /// From the language's code table and the files the choices are learned on.
    pub fn learn<'a>(codes: &Codes, table: usize, files: impl Iterator<Item = &'a Cols>) -> F16Lang {
        let mut l = Learner::default();
        for c in files {
            l.add(c);
        }
        l.finish(codes, table)
    }
    fn numset(&self, set: &[bool]) -> Rc<NumSet> {
        let key = (set.as_ptr() as usize, set.len());
        if let Some(s) = self.sets.borrow().get(&key) {
            return s.clone();
        }
        let mut byte = [false; 256];
        let cut = (self.cut as usize).min(self.by_num.len());
        for (n, b) in byte.iter_mut().enumerate().take(cut) {
            *b = set[self.by_num[n] as usize];
        }
        let tail = (cut..self.by_num.len()).any(|n| set[self.by_num[n] as usize]);
        let by_num = if tail { self.by_num.iter().map(|&k| set[k as usize]).collect() } else { Vec::new() };
        let s = Rc::new(NumSet { byte, tail, by_num });
        self.sets.borrow_mut().insert(key, s.clone());
        s
    }
}

/// One file's format 16 tape, read in place.
pub struct F16<'a> {
    lang: &'a F16Lang,
    /// Byte 0: the version mark.
    mark: u8,
    n: usize,
    codes: Rank8,
    slot: Esc,
    gap: Esc,
    len: Esc,
    ckpt: Packed,
    /// Words the K − 1 gaps after a checkpoint take (a constant of the width).
    chunks: usize,
}

impl<'a> F16<'a> {
    pub fn new(c: &Cols, lang: &'a F16Lang) -> F16<'a> {
        let ids: Vec<u32> = c.code.iter().map(|&k| k as u32).collect();
        // The side lists in parts of N nodes, as stored at rest and read there (E27).
        let n = c.len();
        let codes = Rank8::of_table(&ids, &lang.num, lang.cut, lang.table).use_parts();
        // The leaf slot names the parent on every K-th sibling, and wherever its distance is at most FIT (t39
        // step 23; 0, the E16 rule, now); the width is learned on the E16 values, and nothing more escapes.
        let slot = Esc::new(&sib_slots_fit(c, K, fit_limit(lang.widths[0])), lang.widths[0]).use_parts(n);
        let (g, l) = gaps(c);
        let gap = Esc::new(&g, lang.widths[1]).use_parts(n);
        let len = Esc::new(&l, lang.widths[2]).use_parts(n);
        let starts: Vec<u64> = c.start.iter().step_by(K).copied().collect();
        // The file's width: whole bytes for its length, which a writer knows before the first node (E28).
        let cw = ckpt_width(c.source_len);
        let ckpt = Packed::new_width(&starts, cw);
        let chunks = (K - 1).div_ceil(gap.per());
        assert!(lang.version != 0 && lang.version != MARK_LONG, "a language's choices are marked before a tape is written");
        F16 { lang, mark: lang.version, n: c.len(), codes, slot, gap, len, ckpt, chunks }
    }
    /// Array k's side list in parts of N nodes, one a fixed block (empty for the checkpoints).
    pub fn side_parts_of(&self, k: usize) -> Vec<Vec<u8>> {
        match k {
            0 => self.codes.side_parts(N),
            1 => self.slot.side_parts(self.n, N),
            2 => self.gap.side_parts(self.n, N),
            3 => self.len.side_parts(self.n, N),
            _ => Vec::new(),
        }
    }
    /// Byte 0, the version mark.
    pub fn mark(&self) -> u8 {
        self.mark
    }
    /// Whether a tape marked `mark` can be read with `lang`'s choices: the same revision, else the tape is
    /// rewritten (re-parsed, or converted from its revision) before any read.
    pub fn readable(mark: u8, lang: &F16Lang) -> Result<(), String> {
        if mark == lang.version { Ok(()) } else { Err(format!("tape revision {mark}, this reader's {}", lang.version)) }
    }
    /// Bytes by part: codes, structure (slots), start (gaps and checkpoints), end (lengths).
    pub fn parts(&self) -> [usize; 4] {
        [self.codes.bytes(), self.slot.bytes(), self.gap.bytes() + self.ckpt.bytes(), self.len.bytes()]
    }
    /// Bytes by array: codes, slots, gaps, lengths, checkpoints.
    pub fn arrays(&self) -> [usize; 5] {
        [self.codes.bytes(), self.slot.bytes(), self.gap.bytes(), self.len.bytes(), self.ckpt.bytes()]
    }
    /// Nodes stored past the cutoff, and escaped slots, gaps and lengths.
    pub fn escapes(&self) -> [usize; 4] {
        [self.codes.tail_nodes(), self.slot.escapes(), self.gap.escapes(), self.len.escapes()]
    }
    /// The arrays as stored, each its fixed-width bytes and its side list apart (the at-rest form blocks the
    /// first and keeps the second whole), then the checkpoints: codes, slots, gaps, lengths, checkpoints.
    pub fn streams(&self) -> [(Vec<u8>, Vec<u8>); 5] {
        let ck = crate::repeat::le_bytes(&self.ckpt.words, (self.ckpt_len() * self.ckpt.w as usize).div_ceil(8));
        [self.codes.split(), self.slot.split(self.n), self.gap.split(self.n), self.len.split(self.n), (ck, Vec::new())]
    }
    fn ckpt_len(&self) -> usize {
        self.n.div_ceil(K)
    }
    pub fn widths(&self) -> [u32; 4] {
        [self.lang.widths[0], self.lang.widths[1], self.lang.widths[2], self.ckpt.w]
    }
    /// E29: the tape as the GPU reads it in place (`inplace_gpu`'s `f16_q`): the node count, the codes, the
    /// slots, gaps and lengths, the checkpoints.
    pub fn gpu_image(&self) -> crate::inplace::GpuImage {
        use crate::inplace::Gpu;
        let mut g = crate::inplace::GpuImage { bytes: Vec::new(), params: Vec::new() };
        g.val(self.n as u32);
        self.codes.gpu(&mut g);
        self.slot.gpu(&mut g);
        self.gap.gpu(&mut g);
        self.len.gpu(&mut g);
        self.ckpt.gpu(&mut g);
        g
    }
    #[inline(always)]
    fn number(&self, i: usize) -> u32 {
        self.codes.get(i)
    }
}

impl Reader for F16<'_> {
    fn bytes(&self) -> usize {
        1 + self.parts().iter().sum::<usize>()
    }
    fn id_of(&self, preorder: usize) -> usize {
        preorder
    }
    #[inline(always)]
    fn code(&self, id: usize) -> u32 {
        // The language's code for the checks against the tree; a rule tests numbers (`match_groups`).
        // SAFETY: every number has a code.
        unsafe { *self.lang.by_num.get_unchecked(self.number(id) as usize) }
    }
    #[inline(always)]
    fn subtree_end(&self, id: usize) -> usize {
        let v = self.slot.get(id);
        if v & 1 == 0 { id + (v >> 1) as usize } else { id + 1 }
    }
    #[inline(always)]
    fn parent(&self, id: usize) -> Option<usize> {
        if id == 0 {
            return None;
        }
        // The node before is the parent (tag 0), or a leaf naming the previous sibling (tag 1) or the parent
        // distance (tag 3); at most K − 1 sibling steps.
        let mut j = id;
        loop {
            let v = self.slot.get(j - 1);
            if v & 1 == 0 {
                return Some(j - 1);
            }
            if v & 2 != 0 {
                return Some(j - (v >> 2) as usize);
            }
            j -= (v >> 2) as usize;
        }
    }
    #[inline(always)]
    fn start(&self, id: usize) -> u64 {
        let c = id >> KS;
        self.ckpt.get(c) + self.gap.sum((c << KS) + 1, id + 1, self.chunks)
    }
    #[inline(always)]
    fn end(&self, id: usize) -> u64 {
        self.start(id) + self.len.get(id)
    }
    #[inline(always)]
    fn start_next(&self, id: usize, prev_start: u64) -> u64 {
        if id & (K - 1) == 0 { self.ckpt.get(id >> KS) } else { prev_start + self.gap.get(id) }
    }
    #[inline(always)]
    fn end_with_start(&self, id: usize, start: u64) -> u64 {
        start + self.len.get(id)
    }
    #[inline(always)]
    fn first_child(&self, id: usize) -> Option<usize> {
        (self.slot.get(id) & 1 == 0).then_some(id + 1)
    }
    #[inline(always)]
    fn next_sibling(&self, id: usize, parent_end: usize) -> Option<usize> {
        let e = self.subtree_end(id);
        (e < parent_end).then_some(e)
    }
    fn next(&self, id: usize) -> Option<usize> {
        (id + 1 < self.n).then_some(id + 1)
    }
    fn len(&self) -> usize {
        self.n
    }
    fn prev(&self, id: usize) -> Option<usize> {
        id.checked_sub(1)
    }
    fn rank(&self, id: usize) -> usize {
        id
    }
    fn code_by_rank(&self, r: usize) -> u32 {
        self.code(r)
    }
    /// 64 bytes at a time, each tested by number; a byte past the cutoff is decoded only if the set holds a
    /// code past it.
    fn match_groups(&self, set: &[bool], mut f: impl FnMut(usize, u64)) -> bool {
        let s = self.lang.numset(set);
        let n = self.n;
        let tail_set: Option<&[bool]> = s.tail.then_some(s.by_num.as_slice());
        let cut = self.codes.cut();
        let full = n / 64;
        for g in 0..n.div_ceil(64) {
            let base = g * 64;
            let mut m = 0u64;
            let mut past = 0u64;
            if g < full {
                // A whole group: 64 byte loads and lookups, straight-line (unrolled), as XF's `match_w`.
                // SAFETY: base + 63 < n, and every byte indexes the 256-entry set.
                let bp = unsafe { self.codes.raw().as_ptr().add(base) };
                if tail_set.is_some() {
                    crate::unroll64!(J, {
                        let v = unsafe { *bp.add(J) };
                        m |= u64::from(unsafe { *s.byte.get_unchecked(v as usize) }) << J;
                    });
                    // The prefix bytes, 8 at a time: a byte at or past the cutoff (254 or 255) is all-ones once its
                    // low bit is set (for 254), found exactly, and the high bits gathered into 8 mask bits.
                    let low = if cut <= 254 { 0x0101_0101_0101_0101u64 } else { 0 };
                    for w in 0..8 {
                        // SAFETY: inside the group's 64 bytes.
                        let word = unsafe { std::ptr::read_unaligned(bp.add(w * 8).cast::<u64>()) } | low;
                        let x = !word;
                        let hb = !(((x & LO7) + LO7) | x) & HI;
                        past |= ((hb >> 7).wrapping_mul(0x0102_0408_1020_4080) >> 56) << (w * 8);
                    }
                } else {
                    // No code past the cutoff in the set: the prefix byte's entry is false, nothing to decode.
                    crate::unroll64!(J, {
                        let v = unsafe { *bp.add(J) };
                        m |= u64::from(unsafe { *s.byte.get_unchecked(v as usize) }) << J;
                    });
                }
            } else {
                for j in 0..n - base {
                    let v = self.codes.byte(base + j);
                    // SAFETY: v < 256.
                    m |= u64::from(unsafe { *s.byte.get_unchecked(v as usize) }) << j;
                    past |= u64::from(u32::from(v) >= cut) << j;
                }
            }
            if let Some(ts) = tail_set {
                while past != 0 {
                    let j = past.trailing_zeros() as usize;
                    m |= u64::from(ts[self.number(base + j) as usize]) << j;
                    past &= past - 1;
                }
            }
            f(base, m);
        }
        true
    }
    /// Every span in order: a checkpoint every K nodes, a running sum of gaps between.
    fn spans_bulk(&self, out: &mut Vec<(u64, u64)>) -> bool {
        out.clear();
        out.reserve(self.n);
        let mut s = 0u64;
        for i in 0..self.n {
            s = if i & (K - 1) == 0 { self.ckpt.get(i >> KS) } else { s + self.gap.get(i) };
            out.push((s, s + self.len.get(i)));
        }
        true
    }
    const PREORDER_IDS: bool = true;
}

// ---- E24 at rest: memory, disk, load --------------------------------------------------------------------

use crate::capture::File;
use crate::codec::{self, Codec, Dictionary};
use crate::disk::{Lz4, lz4_decode};
use crate::measure::cost;

pub const STREAMS: [&str; 5] = ["codes", "slots", "gaps", "lengths", "checkpoints"];
/// Nodes a fixed block at rest.
pub const N: usize = crate::code8::PART;
const DICT: usize = 64 << 10;
/// An LZ4 block is kept only if it saves at least a tenth of the raw bytes.
const KEEP: f64 = 0.9;

#[derive(Default)]
pub struct Report {
    pub files: usize,
    pub measured_files: usize,
    pub nodes: u64,
    pub codes: usize,
    pub cut: u32,
    pub widths: [u32; 3],
    /// Measured files, B: by array (codes, slots, gaps, lengths, checkpoints) in memory, and the captured tape's.
    pub mem: [u64; 5],
    pub captured_mem: u64,
    /// Nodes past the cutoff; escaped slots, gaps, lengths.
    pub escapes: [u64; 4],
    /// At rest, B per stream (main + side): zstd 3 + dict, whole file; LZ4 HC + dict, whole; fixed blocks of N
    /// (LZ4 HC + dict or raw, checkpoints raw, side lists one LZ4 block, a 4-byte index entry a block).
    pub zstd: [u64; 5],
    pub lz4: [u64; 5],
    pub blocked: [u64; 5],
    pub index: u64,
    /// The captured tape (the engine's format 16), zstd 3 + dict.
    pub captured_zstd: u64,
    /// Loading, ns: zstd of every stream, whole; every fixed block decoded (LZ4 or copied).
    pub load_zstd: u64,
    pub load_blocks: u64,
    /// One block decoded at random, ns: the mean over every LZ4 block of the codes stream.
    pub one_block: f64,
    pub wrong: u64,
}

/// A file's streams, each main array and side list apart.
type Streams = [(Vec<u8>, Vec<u8>); 5];

pub fn run(files: &[&File]) -> Report {
    let read = |f: &File| crate::captured::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let mut codes = Codes::default();
    for f in files {
        codes.learn(&read(f));
    }
    // The language's choices, and the dictionaries, from the even files; measured on the odd ones.
    let split = files.len() >= 16;
    let cols: Vec<Cols> = files.iter().map(|f| Cols::of_tree(&read(f), &codes)).collect();
    let train_idx: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 0).collect();
    let test_idx: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 1).collect();
    let lang = F16Lang::learn(&codes, grammar_codes(files[0].tag), train_idx.iter().map(|&i| &cols[i])).marked(files[0].tag);
    let mut r = Report { files: files.len(), measured_files: test_idx.len(), codes: codes.table.len(), cut: lang.cut, widths: lang.widths, ..Report::default() };
    let streams = |i: usize| -> Streams { F16::new(&cols[i], &lang).streams() };
    let train: Vec<Streams> = train_idx.iter().map(|&i| streams(i)).collect();
    let dict_of = |k: usize| -> Dictionary {
        if !split {
            return Dictionary::none();
        }
        let samples: Vec<Vec<u8>> = train.iter().map(|s| s[k].0.clone()).filter(|v| !v.is_empty()).collect();
        Dictionary::train(&samples, DICT)
    };
    let dicts: Vec<Dictionary> = (0..5).map(dict_of).collect();
    let captured_dict = if split { Dictionary::train(&train_idx.iter().map(|&i| files[i].image.clone()).collect::<Vec<_>>(), DICT) } else { Dictionary::none() };
    drop(train);
    let mut lz: Vec<Lz4> = dicts.iter().map(|d| Lz4::new(true, &d.bytes)).collect();
    let (mut block_ns, mut blocks_lz) = (0u64, 0u64);
    for &i in &test_idx {
        let c = &cols[i];
        let t = F16::new(c, &lang);
        let n = c.len();
        r.nodes += n as u64;
        for (k, p) in t.arrays().iter().enumerate() {
            r.mem[k] += *p as u64;
        }
        for (k, e) in t.escapes().iter().enumerate() {
            r.escapes[k] += *e as u64;
        }
        r.captured_mem += files[i].image.len() as u64;
        r.captured_zstd += codec::compress(Codec::Zstd, &files[i].image, &captured_dict, 0).len() as u64;
        let s = t.streams();
        // Whole, zstd and LZ4 HC with the dictionary; the load decodes every stream.
        let mut zs: Vec<Vec<u8>> = Vec::new();
        for k in 0..5 {
            let both: Vec<u8> = [s[k].0.as_slice(), s[k].1.as_slice()].concat();
            let z = codec::compress(Codec::Zstd, &both, &dicts[k], 0);
            r.zstd[k] += z.len() as u64;
            r.lz4[k] += lz[k].compress(&both).len() as u64;
            zs.push(z);
        }
        let (back, k) = cost(|| (0..5).map(|k| codec::decompress(Codec::Zstd, &zs[k], &dicts[k], s[k].0.len() + s[k].1.len())).collect::<Vec<_>>());
        r.load_zstd += k.nanos;
        for kk in 0..5 {
            r.wrong += u64::from(back[kk] != [s[kk].0.as_slice(), s[kk].1.as_slice()].concat());
        }
        // Fixed blocks of N nodes: a block of an array covers N nodes' fixed-width bytes (the checkpoints'
        // N / K entries), stored raw (checkpoints, and blocks LZ4 does not shrink by a tenth) or as LZ4 HC
        // over the dictionary. Each block carries its nodes' side-list part after its fixed-width bytes (E26),
        // and an 8-byte index entry (where it is stored, and where its side part goes). The file's first byte
        // is the version mark.
        r.index += 1;
        let stored_mark = t.mark();
        let mut stored: Vec<Vec<(bool, Vec<u8>, usize)>> = Vec::new();
        let mut expect: Vec<Vec<u8>> = vec![Vec::new(); 5];
        for k in 0..5 {
            let main = &s[k].0;
            let per = if k == 4 { (N / K) * (t.widths()[3] as usize / 8) } else { N * t.widths_bytes(k) };
            let parts = t.side_parts_of(k);
            let mut v = Vec::new();
            for (b, blk) in main.chunks(per.max(1)).enumerate() {
                r.index += 8;
                let side: &[u8] = parts.get(b).map_or(&[], Vec::as_slice);
                let plain: Vec<u8> = [blk, side].concat();
                expect[k].extend_from_slice(&plain);
                if k == 4 {
                    r.blocked[k] += plain.len() as u64;
                    v.push((false, plain.clone(), plain.len()));
                    continue;
                }
                let z = lz[k].compress(&plain);
                if (z.len() as f64) < KEEP * plain.len() as f64 {
                    r.blocked[k] += z.len() as u64;
                    v.push((true, z, plain.len()));
                } else {
                    r.blocked[k] += plain.len() as u64;
                    v.push((false, plain.clone(), plain.len()));
                }
            }
            stored.push(v);
        }
        let (got, kk) = cost(|| {
            F16::readable(stored_mark, &lang).expect("a tape of this revision");
            stored.iter().enumerate().map(|(k, v)| {
                let mut out = Vec::with_capacity(s[k].0.len());
                let mut buf = vec![0u8; v.iter().map(|b| b.2).max().unwrap_or(0)];
                for (is_lz, b, len) in v {
                    if *is_lz {
                        let m = lz4_decode(b, &mut buf[..*len], &dicts[k].bytes);
                        out.extend_from_slice(&buf[..m]);
                    } else {
                        out.extend_from_slice(b);
                    }
                }
                out
            }).collect::<Vec<_>>()
        });
        r.load_blocks += kk.nanos;
        for k in 0..5 {
            r.wrong += u64::from(got[k] != expect[k]);
        }
        // One codes block decoded alone, each LZ4 block of the file.
        let mut buf = vec![0u8; stored[0].iter().map(|b| b.2).max().unwrap_or(0)];
        for (is_lz, b, len) in &stored[0] {
            if *is_lz {
                let (m, kk) = cost(|| lz4_decode(b, &mut buf[..*len], &dicts[0].bytes));
                std::hint::black_box(m);
                block_ns += kk.nanos;
                blocks_lz += 1;
            }
        }
    }
    r.one_block = block_ns as f64 / blocks_lz.max(1) as f64;
    r
}

/// One stored block at rest: its array (0–4), LZ4 or raw, where its bytes are in `AtRest::data`, and where its
/// output goes. A block decodes to the array's N nodes, then the part of the array's side list for those nodes
/// (0156 §1.5, cut per fixed block: `code8::side_parts`), which goes to the side list's place.
#[derive(Clone, Copy, Debug)]
pub struct Stored {
    pub array: u8,
    pub lz: bool,
    pub src: u32,
    pub src_len: u32,
    pub out: u32,
    pub out_len: u32,
    pub side_out: u32,
    pub side_len: u32,
}

/// E26: a language's measured files at rest (fixed blocks of N nodes, LZ4 HC over the array's dictionary or
/// raw; checkpoints raw), each block carrying its nodes' side-list part, with the bytes the blocks must decode
/// to: every array's fixed-width part in block order, then every side part.
pub struct AtRest {
    pub files: usize,
    pub nodes: u64,
    /// Arrays 0–4's dictionaries (the checkpoints' is empty).
    pub dicts: Vec<Vec<u8>>,
    pub blocks: Vec<Stored>,
    pub data: Vec<u8>,
    pub expected: Vec<u8>,
    /// Each file's version mark in `data`, and the revision the reader expects.
    pub marks: Vec<(u32, u8)>,
    /// E24's form of the same files: each block without its side part, and each side list one LZ4 block a
    /// file and array, B.
    pub e24_bytes: u64,
}

impl AtRest {
    /// Several languages' blocks as one set, as one dispatch decodes them: each part's offsets moved past the
    /// parts before it, and each block's `array` made its dictionary's index (part × 5 + array).
    pub fn merge(parts: &[AtRest]) -> AtRest {
        let mut m = AtRest { files: 0, nodes: 0, dicts: Vec::new(), blocks: Vec::new(), data: Vec::new(), expected: Vec::new(), marks: Vec::new(), e24_bytes: 0 };
        for (p, a) in parts.iter().enumerate() {
            let (src, out) = (m.data.len() as u32, m.expected.len() as u32);
            m.files += a.files;
            m.nodes += a.nodes;
            m.e24_bytes += a.e24_bytes;
            m.dicts.extend(a.dicts.iter().cloned());
            m.blocks.extend(a.blocks.iter().map(|b| Stored { array: (p * 5) as u8 + b.array, src: b.src + src, out: b.out + out, side_out: b.side_out + out, ..*b }));
            m.marks.extend(a.marks.iter().map(|&(at, v)| (at + src, v)));
            m.data.extend_from_slice(&a.data);
            m.expected.extend_from_slice(&a.expected);
        }
        m
    }
}

pub fn at_rest(files: &[&File]) -> AtRest {
    let read = |f: &File| crate::captured::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let mut codes = Codes::default();
    for f in files {
        codes.learn(&read(f));
    }
    let split = files.len() >= 16;
    let cols: Vec<Cols> = files.iter().map(|f| Cols::of_tree(&read(f), &codes)).collect();
    let train_idx: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 0).collect();
    let test_idx: Vec<usize> = (0..files.len()).filter(|i| !split || i % 2 == 1).collect();
    let lang = F16Lang::learn(&codes, grammar_codes(files[0].tag), train_idx.iter().map(|&i| &cols[i])).marked(files[0].tag);
    let dicts: Vec<Vec<u8>> = (0..5)
        .map(|k| {
            if !split || k == 4 {
                return Vec::new();
            }
            let samples: Vec<Vec<u8>> = train_idx.iter().map(|&i| F16::new(&cols[i], &lang).streams()[k].0.clone()).filter(|v| !v.is_empty()).collect();
            Dictionary::train(&samples, DICT).bytes
        })
        .collect();
    let mut lz: Vec<Lz4> = dicts.iter().map(|d| Lz4::new(true, d)).collect();
    let mut lz_side = Lz4::new(true, &[]);
    let mut a = AtRest { files: test_idx.len(), nodes: 0, dicts: dicts.clone(), blocks: Vec::new(), data: Vec::new(), expected: Vec::new(), marks: Vec::new(), e24_bytes: 0 };
    let mut sides: Vec<u8> = Vec::new();
    for &i in &test_idx {
        let t = F16::new(&cols[i], &lang);
        let n = cols[i].len();
        a.nodes += n as u64;
        // The file's first stored byte: its version mark.
        a.marks.push((a.data.len() as u32, lang.version));
        a.data.push(t.mark());
        let s = t.streams();
        for k in 0..5 {
            if !s[k].1.is_empty() {
                a.e24_bytes += lz_side.compress(&s[k].1).len() as u64;
            }
            let per = if k == 4 { (N / K) * (t.widths()[3] as usize / 8) } else { N * t.widths_bytes(k) };
            let parts = t.side_parts_of(k);
            for (b, blk) in s[k].0.chunks(per.max(1)).enumerate() {
                let side: &[u8] = parts.get(b).map_or(&[], Vec::as_slice);
                let plain: Vec<u8> = [blk, side].concat();
                let z = if k == 4 { None } else { Some(lz[k].compress(&plain)) };
                a.e24_bytes += if k == 4 {
                    blk.len()
                } else {
                    let z0 = lz[k].compress(blk).len();
                    if (z0 as f64) < KEEP * blk.len() as f64 { z0 } else { blk.len() }
                } as u64;
                let (is_lz, stored) = match z {
                    Some(z) if (z.len() as f64) < KEEP * plain.len() as f64 => (true, z),
                    _ => (false, plain),
                };
                a.blocks.push(Stored {
                    array: k as u8,
                    lz: is_lz,
                    src: a.data.len() as u32,
                    src_len: stored.len() as u32,
                    out: a.expected.len() as u32,
                    out_len: blk.len() as u32,
                    side_out: sides.len() as u32,
                    side_len: side.len() as u32,
                });
                a.data.extend_from_slice(&stored);
                a.expected.extend_from_slice(blk);
                sides.extend_from_slice(side);
            }
        }
    }
    let base = a.expected.len() as u32;
    for b in &mut a.blocks {
        b.side_out += base;
    }
    a.expected.extend_from_slice(&sides);
    a
}

impl F16<'_> {
    /// Bytes a node in array k's fixed-width part (codes one; slots, gaps, lengths their width).
    fn widths_bytes(&self, k: usize) -> usize {
        if k == 0 { 1 } else { self.lang.widths[k - 1] as usize / 8 }
    }
}

pub fn print(title: &str, r: &Report) {
    let nf = r.nodes.max(1) as f64;
    let b = |x: u64| format!("{:.3}", x as f64 / nf);
    let pct = |x: u64| format!("{:.2}%", 100.0 * x as f64 / nf);
    let sum = |a: &[u64]| a.iter().sum::<u64>();
    println!("### {title}\n");
    println!(
        "{} files, {} measured ({} nodes){}. {} codes, cutoff {}; widths: slots {}, gaps {}, lengths {} bits. Wrong: {}.\n",
        r.files, r.measured_files, r.nodes, if r.measured_files < r.files { ", numbering, widths and dictionaries from the others" } else { "" },
        r.codes, r.cut, r.widths[0], r.widths[1], r.widths[2], r.wrong
    );
    println!("| B a node | codes | slots | gaps | lengths | checkpoints | total |");
    println!("|---|---|---|---|---|---|---|");
    println!("| in memory | {} | {} | {} | {} | {} | **{}** (captured {}) |", b(r.mem[0]), b(r.mem[1]), b(r.mem[2]), b(r.mem[3]), b(r.mem[4]), b(sum(&r.mem)), b(r.captured_mem));
    println!("| zstd 3 + dict, whole | {} | {} | {} | {} | {} | **{}** (captured {}) |", b(r.zstd[0]), b(r.zstd[1]), b(r.zstd[2]), b(r.zstd[3]), b(r.zstd[4]), b(sum(&r.zstd)), b(r.captured_zstd));
    println!("| LZ4 HC + dict, whole | {} | {} | {} | {} | {} | **{}** |", b(r.lz4[0]), b(r.lz4[1]), b(r.lz4[2]), b(r.lz4[3]), b(r.lz4[4]), b(sum(&r.lz4)));
    println!("| fixed blocks of {N}, LZ4 HC + dict (index {}) | {} | {} | {} | {} | {} | **{}** |", b(r.index), b(r.blocked[0]), b(r.blocked[1]), b(r.blocked[2]), b(r.blocked[3]), b(r.blocked[4]), b(sum(&r.blocked) + r.index));
    println!();
    println!("Past the cutoff {}; escaped slots {}, gaps {}, lengths {}. Load, ns a node: zstd whole {:.2}; every fixed block {:.2}. One codes block decoded at random: {:.0} ns.\n",
        pct(r.escapes[0]), pct(r.escapes[1]), pct(r.escapes[2]), pct(r.escapes[3]), r.load_zstd as f64 / nf, r.load_blocks as f64 / nf, r.one_block);
}
