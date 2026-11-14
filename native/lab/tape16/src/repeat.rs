//! E15 (t39 step 7): "repeated structure gives repeated bits" (0156's requirement), priced.
//!
//! XF keeps its structure slots in `PFor` blocks: each block picks its own width, and an escape slot names its
//! exception by its index in the block. So the same subtree in two blocks can be stored as different bits. Here
//! the slots are also kept at one width with escapes that name nothing (`Esc`: the escape is the all-ones slot,
//! and its value is found in the block's short list of escaped offsets), at the file's width, at the language's
//! width, and at a byte. For each, over a capture's files:
//!
//! - how many nodes of repeated subtrees (8 nodes or more; repeated in the file, or across files) are stored as
//!   the same bits as the subtree's first occurrence;
//! - in-memory bytes a node, and zstd on disk (each file alone; and with a dictionary trained on the other half
//!   of the files, as the store trains them);
//! - what a point read of the slot costs (subtree end, in order and at random).

use crate::codec::{self, Codec, Dictionary};
use crate::inplace::{BLOCK, Codes, Cols, ForLen, Image, PFor, Packed, Pos, bits_for, extract, put, raw};
use crate::measure::{Cost, cost};
use crate::tree::Tree;
use std::collections::HashMap;

/// XF's slot a node: an inner node's size (tag 0); a leaf's, the back distance of the node after it (tag 1).
pub fn xf_slots(c: &Cols) -> Vec<u64> {
    let n = c.len();
    (0..n).map(|i| {
        if c.size[i] > 1 {
            u64::from(c.size[i]) << 1
        } else if i + 1 < n {
            (((i + 1 - c.parent[i + 1] as usize) as u64) << 1) | 1
        } else {
            1
        }
    }).collect()
}

/// One width `w` over the whole array. The all-ones slot is an escape; block b's escapes are
/// `off[start[b]..start[b + 1]]` (offsets in the block, ascending) with their values at the same index of
/// `val`, found by a binary search (8 probes at most). An escape names nothing, so equal runs of values are
/// equal runs of slots.
pub struct Esc {
    w: u32,
    words: Vec<u64>,
    start: Vec<u32>,
    off: Vec<u8>,
    val: Packed,
    /// Constants of the width, not stored (generated code would hold them): the values a word holds, and
    /// the masks that fold a word's fields into their sum (fields of w, 2w, 4w, … bits).
    per: usize,
    folds: Vec<(u32, u64)>,
    /// Each field's low w − 1 bits, over a word's `per` fields (for the exact all-ones test).
    low: u64,
}

impl Image for Esc {
    fn image(&self, out: &mut Vec<u8>) {
        out.push(self.w as u8);
        raw(out, &self.words);
        raw(out, &self.start);
        raw(out, &self.off);
        self.val.image(out);
    }
}

/// The width costing least (every slot at `w` bits, plus each escape's offset byte and value) among those
/// escaping at most `share` of the values.
pub fn esc_width(vals: impl Iterator<Item = u64>, share: f64) -> u32 {
    let mut hist = [0u64; 66];
    let mut n = 0u64;
    for v in vals {
        hist[bits_for(v) as usize] += 1;
        n += 1;
    }
    let top = (0..=64).rev().find(|&b| hist[b] > 0).unwrap_or(0) as u32;
    // A value of exactly 2^w − 1 escapes too; counted as above w here, a small overestimate.
    let (mut best, mut above) = ((u64::MAX, top.max(1)), 0u64);
    for w in (1..=top.max(1)).rev() {
        let cost = n * u64::from(w) + above * (8 + u64::from(top));
        if cost <= best.0 && above as f64 <= share * n as f64 {
            best = (cost, w);
        }
        above += hist[w as usize];
    }
    best.1
}

impl Esc {
    pub fn new(vals: &[u64], w: u32) -> Esc {
        let esc = (1u64 << w) - 1;
        let mut words = vec![0u64; 2];
        let (mut start, mut off, mut val) = (vec![0u32], Vec::new(), Vec::new());
        for (b, chunk) in vals.chunks(BLOCK).enumerate() {
            for (j, &v) in chunk.iter().enumerate() {
                let i = b * BLOCK + j;
                if v >= esc {
                    put(&mut words, i * w as usize, w, esc);
                    off.push(j as u8);
                    val.push(v);
                } else {
                    put(&mut words, i * w as usize, w, v);
                }
            }
            start.push(off.len() as u32);
        }
        put(&mut words, vals.len() * w as usize, 0, 0);
        let per = (64 / w as usize).max(1);
        let mut folds = Vec::new();
        let mut fw = w;
        while (fw as usize) < per * w as usize {
            let mut m = 0u64;
            let mut b = 0;
            while b < 64 {
                m |= (if fw >= 64 { u64::MAX } else { (1u64 << fw) - 1 }) << b;
                b += 2 * fw;
            }
            folds.push((fw, m));
            fw *= 2;
        }
        let mut low = 0u64;
        for f in 0..per {
            low |= ((1u64 << (w - 1)) - 1) << (f * w as usize);
        }
        Esc { w, words, start, off, val: Packed::new(&val), per, folds, low }
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u64 {
        let v = extract(&self.words, i * self.w as usize, self.w);
        if v != (1u64 << self.w) - 1 {
            return v;
        }
        self.escaped(i)
    }
    #[cold]
    fn escaped(&self, i: usize) -> u64 {
        let b = i / BLOCK;
        let (s, e) = (self.start[b] as usize, self.start[b + 1] as usize);
        let j = (i % BLOCK) as u8;
        let k = self.off[s..e].binary_search(&j).expect("an escape is listed");
        self.val.get(s + k)
    }
    pub fn field(&self, i: usize) -> (u32, u64, Option<u64>) {
        let v = extract(&self.words, i * self.w as usize, self.w);
        (self.w, v, (v == (1u64 << self.w) - 1).then(|| self.escaped(i)))
    }
    pub fn bytes(&self) -> usize {
        1 + self.words.len() * 8 + self.start.len() * 4 + self.off.len() + self.val.bytes()
    }
    pub fn escapes(&self) -> usize {
        self.off.len()
    }
    /// The values one word holds.
    pub fn per(&self) -> usize {
        self.per
    }
    /// The sum of values `from..to`, all in one block, read in place a word at a time: each word's fields
    /// are folded into their sum (as popcount folds bits), then each escape in the range adds its value in
    /// place of the all-ones slot.
    #[inline(always)]
    pub fn sum(&self, from: usize, to: usize, chunks: usize) -> u64 {
        // Compiled once per width, as generated code would be (the sweep's `match_w` likewise).
        match self.w {
            3 => self.sum_w::<3>(from, to, chunks),
            4 => self.sum_w::<4>(from, to, chunks),
            5 => self.sum_w::<5>(from, to, chunks),
            6 => self.sum_w::<6>(from, to, chunks),
            7 => self.sum_w::<7>(from, to, chunks),
            8 => self.sum_w::<8>(from, to, chunks),
            9 => self.sum_w::<9>(from, to, chunks),
            10 => self.sum_w::<10>(from, to, chunks),
            _ => self.sum_any(from, to, chunks),
        }
    }
    #[inline(always)]
    fn sum_w<const W: usize>(&self, from: usize, to: usize, chunks: usize) -> u64 {
        let per = 64 / W;
        let low = {
            let mut m = 0u64;
            let mut f = 0;
            while f < per {
                m |= ((1u64 << (W - 1)) - 1) << (f * W);
                f += 1;
            }
            m
        };
        let mut left = to.saturating_sub(from);
        let (mut s, mut any, mut f) = (0u64, 0u64, from);
        for _ in 0..chunks {
            let t = left.min(per);
            let tw = (t * W) as u32;
            let mut v = extract(&self.words, f * W, tw);
            f += t;
            left -= t;
            let fm = crate::inplace::mask(tw);
            let c = !v & fm;
            any |= !((c & low).wrapping_add(low) | c | low) & fm;
            // The fold, unrolled: fields of W, 2W, 4W, … bits summed pairwise until one is left.
            let mut fw = W;
            while fw < per * W {
                let mut m = 0u64;
                let mut b = 0;
                while b < 64 {
                    m |= ((1u64 << fw) - 1) << b;
                    b += 2 * fw;
                }
                v = (v & m).wrapping_add((v >> fw) & m);
                fw *= 2;
            }
            s = s.wrapping_add(v);
        }
        if any != 0 {
            s = s.wrapping_add(self.escaped_in(from, to));
        }
        s
    }
    /// What the escapes in `from..to` (one block) add over their all-ones slots.
    #[cold]
    fn escaped_in(&self, from: usize, to: usize) -> u64 {
        let b = from / BLOCK;
        let (bs, be) = (self.start[b] as usize, self.start[b + 1] as usize);
        let list = &self.off[bs..be];
        let (lo, hi) = ((from % BLOCK) as u8, ((to - 1) % BLOCK) as u8);
        let (a, z) = (list.partition_point(|&o| o < lo), list.partition_point(|&o| o <= hi));
        let esc = (1u64 << self.w) - 1;
        (a..z).map(|k| self.val.get(bs + k) - esc).sum()
    }
    fn sum_any(&self, from: usize, to: usize, chunks: usize) -> u64 {
        // Wrapping arithmetic throughout: the lab's profile checks overflow, generated code would not.
        let w = self.w as usize;
        let mut left = to.saturating_sub(from);
        debug_assert!(left <= chunks * self.per, "the range fits the chunks");
        let folds = &self.folds[..];
        // `chunks` words' worth of fields, a fixed count for the caller's range (a word of no fields reads 0).
        let (mut s, mut esc, mut f) = (0u64, 0u32, from);
        for _ in 0..chunks {
            let t = left.min(self.per);
            let tw = t.wrapping_mul(w);
            let mut v = extract(&self.words, f.wrapping_mul(w), tw as u32);
            f = f.wrapping_add(t);
            left = left.wrapping_sub(t);
            // Exact: a field's top bit is set when the field is all ones (complemented, all zeros).
            let fm = crate::inplace::mask(tw as u32);
            let c = !v & fm;
            let y = !((c & self.low).wrapping_add(self.low) | c | self.low) & fm;
            esc = esc.wrapping_add(y.count_ones());
            for &(fw, m) in folds {
                v = (v & m).wrapping_add((v >> fw) & m);
            }
            s = s.wrapping_add(v);
        }
        if esc > 0 {
            let b = from / BLOCK;
            let bs = self.start[b] as usize;
            let be = self.start[b + 1] as usize;
            let list = &self.off[bs..be];
            let (lo, hi) = ((from % BLOCK) as u8, ((to - 1) % BLOCK) as u8);
            let a = list.partition_point(|&o| o < lo);
            let z = list.partition_point(|&o| o <= hi);
            let esc = (1u64 << self.w) - 1;
            for k in a..z {
                s += self.val.get(bs + k) - esc;
            }
        }
        s
    }
}

/// The structure (codes and slots) a node at a time, as one variant stores it.
enum Slots {
    PFor(PFor),
    Esc(Esc),
}

impl Slots {
    fn field(&self, i: usize) -> (u32, u64, Option<u64>) {
        match self {
            Slots::PFor(p) => p.field(i),
            Slots::Esc(e) => e.field(i),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Slots::PFor(p) => p.bytes(),
            Slots::Esc(e) => e.bytes(),
        }
    }
    fn image(&self, out: &mut Vec<u8>) {
        match self {
            Slots::PFor(p) => p.image(out),
            Slots::Esc(e) => e.image(out),
        }
    }
}

struct Built {
    code: Packed,
    slot: Slots,
}

const VARIANTS: [&str; NV] = [
    "XF now: codes at the file's width, slots by PFOR blocks",
    "one width a file: codes and slots at the file's width, escapes listed",
    "one width a language: codes and slots at the language's width",
    "bytes: codes and slots at the language's width rounded up to bytes",
    "one width a language, chosen to escape at most 2% of the slots",
];

const NV: usize = 5;

fn build(v: usize, c: &Cols, slots: &[u64], lang: (u32, u32, u32)) -> Built {
    let file_code = Packed::new(&c.code);
    match v {
        0 => Built { code: file_code, slot: Slots::PFor(PFor::new(slots, false)) },
        1 => Built { code: file_code, slot: Slots::Esc(Esc::new(slots, esc_width(slots.iter().copied(), 1.0))) },
        2 => Built { code: Packed::new_width(&c.code, lang.0), slot: Slots::Esc(Esc::new(slots, lang.1)) },
        3 => Built { code: Packed::new_width(&c.code, lang.0.next_multiple_of(8)), slot: Slots::Esc(Esc::new(slots, lang.1.next_multiple_of(8))) },
        _ => Built { code: Packed::new_width(&c.code, lang.0), slot: Slots::Esc(Esc::new(slots, lang.2)) },
    }
}

/// Each node's subtree hash (its code, its size and its children's hashes in order), from the leaves up.
fn subtree_hashes(c: &Cols) -> Vec<u64> {
    let n = c.len();
    let mix = |x: u64| {
        let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut h = vec![0u64; n];
    for i in (0..n).rev() {
        let mut acc = mix(c.code[i] << 32 | u64::from(c.size[i]));
        let end = i + c.size[i] as usize;
        let mut j = i + 1;
        while j < end {
            acc = mix(acc.rotate_left(17) ^ h[j]);
            j += c.size[j] as usize;
        }
        h[i] = acc;
    }
    h
}

pub const MIN_NODES: usize = 8;

/// Repeated-subtree coverage: nodes in repeats (within a file, across files) and, per variant, those stored as
/// the same bits as the first occurrence. A subtree's last node's slot names the node after the subtree, so
/// it is left out of the comparison.
#[derive(Default, Clone, Copy)]
pub struct Cover {
    pub nodes: u64,
    pub same: [u64; NV],
}

pub struct Report {
    pub files: usize,
    pub nodes: u64,
    pub within: Cover,
    pub across: Cover,
    /// In memory, bytes: codes, slots (per variant); positions (shared).
    pub mem: [[u64; 2]; NV],
    pub pos: u64,
    /// zstd level 3, each file alone: codes + slots (per variant); positions.
    pub disk: [u64; NV],
    pub disk_pos: u64,
    /// zstd level 3 with a 112 KB dictionary trained on the even files, over the odd files.
    pub dict: [u64; NV],
    pub dict_pos: u64,
    pub dict_nodes: u64,
    /// Positions as each start's gap to the previous start and each length, four bytes each (not readable in
    /// place: a ceiling for what relative positions would give on disk), alone and with a dictionary.
    pub rel: [u64; 2],
    pub escapes: [u64; NV],
    pub widths: (u32, u32, u32),
    /// Slot reads, ns a read: in order and at random, per variant (over the largest file).
    pub reads: [[f64; 2]; NV],
}

pub fn run(trees: &[Tree], codes: &Codes) -> Report {
    let cols: Vec<Cols> = trees.iter().map(|t| Cols::of_tree(t, codes)).collect();
    let slots: Vec<Vec<u64>> = cols.iter().map(xf_slots).collect();
    let lang = (codes.width(), esc_width(slots.iter().flatten().copied(), 1.0), esc_width(slots.iter().flatten().copied(), 0.02));
    let mut r = Report {
        files: trees.len(),
        nodes: cols.iter().map(|c| c.len() as u64).sum(),
        within: Cover::default(),
        across: Cover::default(),
        mem: [[0; 2]; NV],
        pos: 0,
        disk: [0; NV],
        disk_pos: 0,
        dict: [0; NV],
        dict_pos: 0,
        dict_nodes: 0,
        rel: [0; 2],
        escapes: [0; NV],
        widths: lang,
        reads: [[0.0; 2]; NV],
    };
    if std::env::var_os("E15_ESC").is_some() {
        // Which slots escape at the language's width: leaves (tag 1) or inner nodes (tag 0).
        let esc = (1u64 << lang.1) - 1;
        let (mut leaf, mut inner) = (0u64, 0u64);
        for v in slots.iter().flatten().filter(|&&v| v >= esc) {
            if v & 1 == 1 { leaf += 1 } else { inner += 1 }
        }
        eprintln!("escapes at {} bits: {leaf} leaf slots, {inner} inner", lang.1);
    }
    let built: Vec<Vec<Built>> = cols.iter().zip(&slots).map(|(c, s)| (0..NV).map(|v| build(v, c, s, lang)).collect()).collect();
    // The images: structure per variant, positions once (XF's frame of reference + length).
    let image = |b: &Built| {
        let mut out = Vec::new();
        b.code.image(&mut out);
        b.slot.image(&mut out);
        out
    };
    let pos: Vec<Vec<u8>> = cols.iter().map(|c| {
        let mut out = Vec::new();
        ForLen::build(c).image(&mut out);
        out
    }).collect();
    let rel: Vec<Vec<u8>> = cols.iter().map(|c| {
        let mut out = Vec::with_capacity(c.len() * 8);
        let mut prev = 0u64;
        for i in 0..c.len() {
            let d = c.start[i] as i64 - prev as i64;
            out.extend_from_slice(&(((d << 1) ^ (d >> 63)) as u32).to_le_bytes());
            out.extend_from_slice(&((c.end[i] - c.start[i]) as u32).to_le_bytes());
            prev = c.start[i];
        }
        out
    }).collect();
    let z = |d: &[u8], dict: &Dictionary| codec::compress(Codec::Zstd, d, dict, 0).len() as u64;
    let none = Dictionary::none();
    for (f, b) in built.iter().enumerate() {
        for v in 0..NV {
            r.mem[v][0] += b[v].code.bytes() as u64;
            r.mem[v][1] += b[v].slot.bytes() as u64;
            r.disk[v] += z(&image(&b[v]), &none);
            if let Slots::Esc(e) = &b[v].slot {
                r.escapes[v] += e.escapes() as u64;
            }
        }
        r.pos += pos[f].len() as u64;
        r.disk_pos += z(&pos[f], &none);
        r.rel[0] += z(&rel[f], &none);
    }
    if trees.len() >= 16 {
        for v in 0..NV {
            let train: Vec<Vec<u8>> = built.iter().step_by(2).map(|b| image(&b[v])).collect();
            let d = Dictionary::train(&train, 112 << 10);
            r.dict[v] = built.iter().skip(1).step_by(2).map(|b| z(&image(&b[v]), &d)).sum();
        }
        let train: Vec<Vec<u8>> = pos.iter().step_by(2).cloned().collect();
        let d = Dictionary::train(&train, 112 << 10);
        r.dict_pos = pos.iter().skip(1).step_by(2).map(|p| z(p, &d)).sum();
        let train: Vec<Vec<u8>> = rel.iter().step_by(2).cloned().collect();
        let d = Dictionary::train(&train, 112 << 10);
        r.rel[1] = rel.iter().skip(1).step_by(2).map(|p| z(p, &d)).sum();
        r.dict_nodes = cols.iter().skip(1).step_by(2).map(|c| c.len() as u64).sum();
    }
    // Repeats: the first occurrence of each subtree hash, maximal subtrees only (a repeat's inside is skipped).
    let mut first: HashMap<u64, (usize, usize)> = HashMap::new();
    for (f, c) in cols.iter().enumerate() {
        let h = subtree_hashes(c);
        let n = c.len();
        let mut i = 0;
        while i < n {
            let size = c.size[i] as usize;
            if size < MIN_NODES {
                i += 1;
                continue;
            }
            match first.get(&h[i]) {
                Some(&(g, k)) if cols[g].code[k..k + size] == c.code[i..i + size] && cols[g].size[k..k + size] == c.size[i..i + size] => {
                    let cover = if g == f { &mut r.within } else { &mut r.across };
                    cover.nodes += size as u64;
                    for v in 0..NV {
                        let (a, b) = (&built[g][v], &built[f][v]);
                        let same_code = (0..size).all(|d| (a.code.w, a.code.get(k + d)) == (b.code.w, b.code.get(i + d)));
                        let same_slot = (0..size - 1).all(|d| a.slot.field(k + d) == b.slot.field(i + d));
                        if same_code && same_slot {
                            cover.same[v] += size as u64;
                        }
                    }
                    i += size;
                }
                Some(_) => i += 1,
                None => {
                    first.insert(h[i], (f, i));
                    i += 1;
                }
            }
        }
    }
    // Slot reads on the largest file: subtree end in order, then at random.
    let big = (0..cols.len()).max_by_key(|&f| cols[f].len()).unwrap_or(0);
    let n = cols[big].len();
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for i in (1..n).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let seq: Vec<usize> = (0..n).collect();
    for v in 0..NV {
        for (q, ids) in [&seq, &order].into_iter().enumerate() {
            let mut best = u64::MAX;
            for _ in 0..5 {
                let (s, c): (u64, Cost) = cost(|| {
                    let mut s = 0u64;
                    match &built[big][v].slot {
                        Slots::PFor(p) => ids.iter().for_each(|&i| s = s.wrapping_add(end_of(p.get(i), i))),
                        Slots::Esc(e) => ids.iter().for_each(|&i| s = s.wrapping_add(end_of(e.get(i), i))),
                    }
                    s
                });
                std::hint::black_box(s);
                best = best.min(c.nanos);
            }
            r.reads[v][q] = best as f64 / n.max(1) as f64;
        }
    }
    r
}

/// XF's subtree end from its slot.
#[inline(always)]
fn end_of(v: u64, i: usize) -> u64 {
    let t = v & 1;
    i as u64 + (((v >> 1) & t.wrapping_sub(1)) | t)
}

pub fn print(path: &str, r: &Report) {
    let n = r.nodes as f64;
    println!("{path}: {} files, {} nodes; language widths: codes {} bits, slots {} bits ({} with escapes under 2%)\n", r.files, r.nodes, r.widths.0, r.widths.1, r.widths.2);
    println!("Repeated subtrees ({MIN_NODES} nodes or more, maximal): {:.1}% of nodes repeat a subtree earlier in their file, {:.1}% one in an earlier file.\n",
        100.0 * r.within.nodes as f64 / n, 100.0 * r.across.nodes as f64 / n);
    println!("| | {} |", (0..NV).map(|v| format!("V{v}")).collect::<Vec<_>>().join(" | "));
    println!("|---|{}", "---|".repeat(NV));
    let row = |name: &str, f: &dyn Fn(usize) -> String| println!("| {name} | {} |", (0..NV).map(f).collect::<Vec<_>>().join(" | "));
    row("repeats in the file stored as the same bits", &|v| format!("{:.1}%", 100.0 * r.within.same[v] as f64 / r.within.nodes.max(1) as f64));
    row("repeats across files stored as the same bits", &|v| format!("{:.1}%", 100.0 * r.across.same[v] as f64 / r.across.nodes.max(1) as f64));
    row("in memory, codes + slots, B a node", &|v| format!("{:.3} + {:.3} = {:.3}", r.mem[v][0] as f64 / n, r.mem[v][1] as f64 / n, (r.mem[v][0] + r.mem[v][1]) as f64 / n));
    row("escaped slots", &|v| if v == 0 { String::new() } else { format!("{:.2}%", 100.0 * r.escapes[v] as f64 / n) });
    row("zstd, each file alone, B a node", &|v| format!("{:.3}", r.disk[v] as f64 / n));
    if r.dict_nodes > 0 {
        row("zstd with a dictionary (odd files), B a node", &|v| format!("{:.3}", r.dict[v] as f64 / r.dict_nodes as f64));
    }
    row("slot read in order, ns", &|v| format!("{:.2}", r.reads[v][0]));
    row("slot read at random, ns", &|v| format!("{:.2}", r.reads[v][1]));
    println!();
    for (v, name) in VARIANTS.iter().enumerate() {
        println!("- V{v}: {name}");
    }
    println!("\nPositions (frame of reference + length, the same in every variant): in memory {:.3} B a node, zstd alone {:.3}{}.",
        r.pos as f64 / n, r.disk_pos as f64 / n,
        if r.dict_nodes > 0 { format!(", with a dictionary {:.3}", r.dict_pos as f64 / r.dict_nodes as f64) } else { String::new() });
    println!("Positions relative (each start's gap to the previous start, and the length; not readable in place, a ceiling): zstd alone {:.3}{}.",
        r.rel[0] as f64 / n,
        if r.dict_nodes > 0 { format!(", with a dictionary {:.3}", r.rel[1] as f64 / r.dict_nodes as f64) } else { String::new() });
}

/// A repeat: the subtree at node `i` of file `f` (`size` nodes) has the codes and sizes of the one at node
/// `k` of file `g`, its first occurrence. Maximal: a repeat's inside is not visited.
#[derive(Clone, Copy)]
pub struct Occ {
    pub g: usize,
    pub k: usize,
    pub f: usize,
    pub i: usize,
    pub size: usize,
}

pub fn occurrences(cols: &[Cols]) -> Vec<Occ> {
    let mut first: HashMap<u64, (usize, usize)> = HashMap::new();
    let mut out = Vec::new();
    for (f, c) in cols.iter().enumerate() {
        let h = subtree_hashes(c);
        let n = c.len();
        let mut i = 0;
        while i < n {
            let size = c.size[i] as usize;
            if size < MIN_NODES {
                i += 1;
                continue;
            }
            match first.get(&h[i]) {
                Some(&(g, k)) if cols[g].code[k..k + size] == c.code[i..i + size] && cols[g].size[k..k + size] == c.size[i..i + size] => {
                    out.push(Occ { g, k, f, i, size });
                    i += size;
                }
                Some(_) => i += 1,
                None => {
                    first.insert(h[i], (f, i));
                    i += 1;
                }
            }
        }
    }
    out
}
