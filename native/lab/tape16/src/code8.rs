//! E20 (t39 step 12): codes in 8 bits, overloaded by the adjacent code.
//!
//! A language's codes are its distinct (kind, field, flags, supertypes) tuples: Python has about 300, so 9 bits.
//! Here each code gets one stored byte `f(code)` in 0..=254, the same in every context, so a repeated subtree
//! is still the same bytes. Two codes may share a byte. The reader tells them apart by the byte before it in
//! the same array (the previous node in preorder: its parent for a first child, else the last node of the
//! previous sibling's subtree), through a language table `T[previous byte][byte] -> code` of 256 × 256 u16
//! (128 KiB). Where two codes sharing a byte both follow the same previous byte, the rarer one there is stored
//! as the escape 255 with its code in the block's side list (as `Esc`'s escapes). A node after an escape reads
//! the context 255, so no read needs more than the two bytes.
//!
//! The shared bytes are chosen greedily, then improved by moving one code at a time to the byte where it
//! escapes least, on the even files; escapes are counted on the odd ones (a new file meets pairs the table has
//! not seen). The same is measured with the parent's byte as the context, which a read pays a parent for.

use crate::capture::File;
use crate::codec::{self, Codec, Dictionary};
use crate::disk::Lz4;
use crate::inplace::{BLOCK, Codes, Cols, Packed};
use crate::measure::cost;
use crate::repeat::{le_bytes, occurrences};
use crate::tree::NONE;
use std::collections::{HashMap, HashSet};

const ESC: u8 = 255;
/// In the one-byte table: the byte is shared (or the escape), so the previous byte decides.
const SHARED: u16 = u16::MAX;
/// The context of a node with none before it (the root).
const START: u32 = u32::MAX;

/// Which earlier node gives a node's context.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    Prev,
    Parent,
}

fn ctx_of(c: &Cols, i: usize, ctx: Ctx) -> Option<usize> {
    match ctx {
        Ctx::Prev => i.checked_sub(1),
        Ctx::Parent => (c.parent[i] != NONE).then_some(c.parent[i] as usize),
    }
}

/// The byte each code is stored as, and the decode table.
pub struct Assign {
    pub f: Vec<u8>,
    /// `t[s * 256 + v]`: the code byte v stands for after byte s. Built at u32 (256 KiB); the reads use a u16
    /// copy (128 KiB), except `Code8::get`, kept as the first measurement.
    pub t: Vec<u32>,
}

/// Pair counts: (previous code or START, code) -> nodes.
fn pairs(cols: &[&Cols], ctx: Ctx) -> HashMap<(u32, u32), u64> {
    let mut m = HashMap::new();
    for c in cols {
        for i in 0..c.len() {
            let p = ctx_of(c, i, ctx).map_or(START, |j| c.code[j] as u32);
            *m.entry((p, c.code[i] as u32)).or_insert(0) += 1;
        }
    }
    m
}

/// The escapes a byte's codes cost, per context byte: the nodes of every code there but the most frequent.
#[derive(Default, Clone)]
struct Slot {
    /// context byte -> (sum, max)
    by: HashMap<u16, (u64, u64)>,
    codes: Vec<u32>,
}

/// Chooses `f` over the pairs: every code in frequency order to the byte where it adds fewest escapes, then
/// `passes` rounds of moving each code to its best byte with the contexts recomputed from the current `f`.
pub fn assign(ncodes: usize, pairs: &HashMap<(u32, u32), u64>, passes: usize) -> Assign {
    let mut freq = vec![0u64; ncodes];
    let mut by_code: Vec<Vec<(u32, u64)>> = vec![Vec::new(); ncodes];
    for (&(p, c), &n) in pairs {
        freq[c as usize] += n;
        by_code[c as usize].push((p, n));
    }
    let mut order: Vec<usize> = (0..ncodes).collect();
    order.sort_by_key(|&c| std::cmp::Reverse(freq[c]));
    // A code's context byte: its predecessor's byte under the current f; before a predecessor is placed, a
    // context of its own (256 + code), so it merges with nothing.
    let ctx_byte = |p: u32, f: &[Option<u8>]| -> u16 {
        if p == START { u16::from(ESC) } else { f[p as usize].map_or(256 + p as u16, u16::from) }
    };
    let added = |slot: &Slot, ctxs: &[(u16, u64)]| -> u64 {
        let mut d = 0u64;
        for &(s, n) in ctxs {
            let (sum, max) = slot.by.get(&s).copied().unwrap_or((0, 0));
            d += (sum + n - max.max(n)) - (sum - max);
        }
        d
    };
    let place = |slot: &mut Slot, c: u32, ctxs: &[(u16, u64)]| {
        for &(s, n) in ctxs {
            let e = slot.by.entry(s).or_insert((0, 0));
            e.0 += n;
            e.1 = e.1.max(n);
        }
        slot.codes.push(c);
    };
    let ctxs_of = |c: usize, f: &[Option<u8>]| -> Vec<(u16, u64)> {
        let mut m: HashMap<u16, u64> = HashMap::new();
        for &(p, n) in &by_code[c] {
            *m.entry(ctx_byte(p, f)).or_insert(0) += n;
        }
        m.into_iter().collect()
    };
    let weight = |slot: &Slot| -> u64 { slot.codes.iter().map(|&c| freq[c as usize]).sum() };
    let mut f: Vec<Option<u8>> = vec![None; ncodes];
    let mut slots = vec![Slot::default(); 255];
    for &c in &order {
        let ctxs = ctxs_of(c, &f);
        // Fewest escapes; then the byte whose codes are rarest, so frequent codes keep a byte of their own.
        let v = (0..255).min_by_key(|&v| (added(&slots[v], &ctxs), weight(&slots[v]))).expect("255 bytes");
        place(&mut slots[v], c as u32, &ctxs);
        f[c] = Some(v as u8);
    }
    for _ in 0..passes {
        // Rebuild every slot under the current f, then move each code (rarest first) to its best byte.
        let rebuild = |f: &[Option<u8>]| {
            let mut slots = vec![Slot::default(); 255];
            for c in 0..ncodes {
                if let Some(v) = f[c] {
                    let ctxs = ctxs_of(c, f);
                    place(&mut slots[v as usize], c as u32, &ctxs);
                }
            }
            slots
        };
        let mut slots = rebuild(&f);
        for &c in order.iter().rev() {
            let ctxs = ctxs_of(c, &f);
            let cur = f[c].expect("placed") as usize;
            // Remove c from its slot (exactly: rebuilt from the slot's other codes).
            slots[cur].codes.retain(|&x| x != c as u32);
            let others = std::mem::take(&mut slots[cur].codes);
            slots[cur] = Slot::default();
            for &o in &others {
                let oc = ctxs_of(o as usize, &f);
                place(&mut slots[cur], o, &oc);
            }
            let v = (0..255).min_by_key(|&v| (added(&slots[v], &ctxs), weight(&slots[v]), u64::from(v != cur))).expect("255 bytes");
            place(&mut slots[v], c as u32, &ctxs);
            f[c] = Some(v as u8);
        }
    }
    let f: Vec<u8> = f.into_iter().map(|v| v.expect("placed")).collect();
    // The table, from the pairs under f: per (context byte, byte), the most frequent code.
    // Several predecessors share a byte s: their counts add per code.
    let mut best: HashMap<(u8, u8), (u64, u32)> = HashMap::new();
    let mut sums: HashMap<(u8, u8, u32), u64> = HashMap::new();
    for (&(p, c), &n) in pairs {
        let s = if p == START { ESC } else { f[p as usize] };
        *sums.entry((s, f[c as usize], c)).or_insert(0) += n;
    }
    for (&(s, v, c), &n) in &sums {
        let e = best.entry((s, v)).or_insert((0, u32::MAX));
        if n > e.0 || (n == e.0 && c < e.1) {
            *e = (n, c);
        }
    }
    // A context the table has not seen (and the one after an escape, 255) reads each byte as its most
    // frequent code: an unseen pair then escapes only if it is not that code.
    let mut default = vec![(0u64, u32::MAX); 256];
    for (c, &v) in f.iter().enumerate() {
        if freq[c] > default[v as usize].0 || default[v as usize].1 == u32::MAX {
            default[v as usize] = (freq[c], c as u32);
        }
    }
    let mut t: Vec<u32> = (0..256 * 256).map(|i| default[i % 256].1).collect();
    for (&(s, v), &(_, c)) in &best {
        if s != ESC {
            t[s as usize * 256 + v as usize] = c;
        }
    }
    Assign { f, t }
}

/// A file's codes stored under `a`: the bytes, and the escapes (node, code), by the decode rule (the context
/// is the stored byte before, so an escape's successor reads 255).
fn store(c: &Cols, a: &Assign, ctx: Ctx) -> (Vec<u8>, Vec<(usize, u32)>) {
    let n = c.len();
    let mut bytes = vec![0u8; n];
    let mut esc = Vec::new();
    for i in 0..n {
        let s = ctx_of(c, i, ctx).map_or(ESC, |j| bytes[j]);
        let code = c.code[i] as u32;
        let v = a.f[code as usize];
        if a.t[s as usize * 256 + v as usize] == code {
            bytes[i] = v;
        } else {
            bytes[i] = ESC;
            esc.push((i, code));
        }
    }
    (bytes, esc)
}

/// A file's 8-bit codes as read in place: bytes, the language table, and the escapes a block of BLOCK nodes
/// at a time (start a block, offsets in it, codes), as `Esc` keeps them.
pub struct Code8<'a> {
    bytes: Vec<u8>,
    t: &'a [u32],
    /// The same table at u16 (128 KiB), and a byte's code where no other code shares it, else SHARED.
    t16: Vec<u16>,
    one: [u16; 256],
    /// Bytes another code shares (and the escape), as a 256-bit set: with codes numbered so that a byte one
    /// code alone stands for is that code's number, a read is the byte unless its bit is set.
    shared_set: [u64; 4],
    start: Vec<u32>,
    off: Vec<u8>,
    val: Vec<u16>,
}

impl<'a> Code8<'a> {
    fn new(bytes: Vec<u8>, esc: &[(usize, u32)], t: &'a [u32]) -> Code8<'a> {
        let blocks = bytes.len().div_ceil(BLOCK);
        let mut start = vec![0u32; blocks + 1];
        let (mut off, mut val) = (Vec::new(), Vec::new());
        for &(i, c) in esc {
            start[i / BLOCK + 1] += 1;
            off.push((i % BLOCK) as u8);
            val.push(c as u16);
        }
        for b in 0..blocks {
            start[b + 1] += start[b];
        }
        let t16: Vec<u16> = t.iter().map(|&c| c as u16).collect();
        // A byte one code alone stands for reads as that code in every context.
        let mut one = [SHARED; 256];
        for v in 0..256 {
            let first = t16[v];
            if (0..256).all(|s| t16[s * 256 + v] == first) {
                one[v] = first;
            }
        }
        let mut shared_set = [0u64; 4];
        for v in 0..256 {
            if one[v] == SHARED {
                shared_set[v / 64] |= 1 << (v % 64);
            }
        }
        Code8 { bytes, t, t16, one, start, off, val, shared_set }
    }
    /// The stored byte alone: what a sweep or a kind test compares.
    #[inline(always)]
    pub fn byte(&self, i: usize) -> u8 {
        // SAFETY: callers pass i < len.
        unsafe { *self.bytes.get_unchecked(i) }
    }
    /// The code: the 256-entry table for a byte one code alone stands for (all but 1–3% of nodes), else the
    /// previous byte and the 128 KiB table.
    #[inline(always)]
    pub fn get_fast(&self, i: usize) -> u32 {
        let v = self.byte(i);
        // SAFETY: v < 256, and the 2-D table has 256 × 256 entries.
        let c = unsafe { *self.one.get_unchecked(v as usize) };
        if c != SHARED {
            return u32::from(c);
        }
        self.shared(i, v)
    }
    #[cold]
    #[inline(never)]
    fn shared(&self, i: usize, v: u8) -> u32 {
        if v == ESC {
            return self.escaped(i);
        }
        let s = if i == 0 { ESC } else { self.byte(i - 1) };
        u32::from(self.t16[(s as usize) << 8 | v as usize])
    }
    /// With codes renumbered so an unshared byte is its own code's number: the byte, unless it is shared (the
    /// number returned is then the table's code, which a renumbering would map past 255; here the lab's own
    /// numbering, so only the cost is measured).
    #[inline(always)]
    pub fn get_ident(&self, i: usize) -> u32 {
        let v = self.byte(i);
        // SAFETY: v / 64 < 4.
        if unsafe { *self.shared_set.get_unchecked(v as usize >> 6) } >> (v & 63) & 1 == 0 {
            return u32::from(v);
        }
        self.shared(i, v)
    }
    /// Every read through the 2-D table at u16 (128 KiB), unchecked.
    #[inline(always)]
    pub fn get_t16(&self, i: usize) -> u32 {
        let v = self.byte(i);
        if v == ESC {
            return self.escaped(i);
        }
        let s = if i == 0 { ESC } else { self.byte(i - 1) };
        // SAFETY: both bytes are below 256.
        u32::from(unsafe { *self.t16.get_unchecked((s as usize) << 8 | v as usize) })
    }
    #[inline(always)]
    pub fn get(&self, i: usize) -> u32 {
        let v = self.bytes[i];
        if v == ESC {
            return self.escaped(i);
        }
        let s = if i == 0 { ESC } else { self.bytes[i - 1] };
        self.t[(s as usize) << 8 | v as usize]
    }
    #[cold]
    fn escaped(&self, i: usize) -> u32 {
        let b = i / BLOCK;
        let (s, e) = (self.start[b] as usize, self.start[b + 1] as usize);
        let k = self.off[s..e].binary_search(&((i % BLOCK) as u8)).expect("an escape is listed");
        u32::from(self.val[s + k])
    }
    pub fn side_bytes(&self) -> usize {
        self.start.len() * 4 + self.off.len() + self.val.len() * 2
    }
}

pub struct Report {
    pub files: usize,
    pub nodes: u64,
    pub codes: usize,
    pub kinds: usize,
    pub kind_field: usize,
    pub kind_rest: usize,
    /// Bytes holding two codes or more; codes that share a byte.
    pub shared_bytes: usize,
    pub shared_codes: usize,
    /// Escaped nodes on the measured files: previous byte, parent byte; and on the files it was chosen on.
    pub escapes: [u64; 2],
    pub escapes_train: u64,
    pub train_nodes: u64,
    pub measured_nodes: u64,
    /// Nodes whose byte is shared with another code (a sweep for a code set confirms them by the table).
    pub nodes_shared: u64,
    pub side_bytes: u64,
    /// Of the nodes of repeated subtrees (8 nodes or more), those whose stored bytes equal the first
    /// occurrence's (the subtree's root excluded: its context lies outside it).
    pub repeat_nodes: u64,
    pub repeat_same: u64,
    pub wrong: u64,
    /// ns a read, at random then in order: the byte alone; the code by the one-byte table with the 2-D one
    /// for shared bytes; every read through the 2-D table at u16; at u32 (the first measurement); and the
    /// language's width bit-packed.
    pub ns: [[f64; 2]; 6],
    /// On disk, B a node (measured files, dictionary from the others): zstd 3, LZ4 HC 9: 8-bit (bytes + side),
    /// language width bit-packed, 16 bits.
    pub disk: [[f64; 2]; 3],
}

pub fn run(files: &[&File]) -> Report {
    let read = |f: &File| crate::v15::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let trees: Vec<_> = files.iter().map(|f| read(f)).collect();
    let mut codes = Codes::default();
    for t in &trees {
        codes.learn(t);
    }
    let cols: Vec<Cols> = trees.iter().map(|t| Cols::of_tree(t, &codes)).collect();
    drop(trees);
    let distinct = |key: &dyn Fn(&(u16, u8, u8, Vec<u16>)) -> (u16, u8, u8, Vec<u16>)| codes.table.iter().map(key).collect::<HashSet<_>>().len();
    let kinds = distinct(&|k| (k.0, 0, 0, Vec::new()));
    let kind_field = distinct(&|k| (k.0, k.1, 0, Vec::new()));
    let kind_rest = distinct(&|k| (k.0, 0, k.2, k.3.clone()));
    let split = files.len() >= 4;
    let (train, test): (Vec<&Cols>, Vec<&Cols>) = if split {
        (cols.iter().step_by(2).collect(), cols.iter().skip(1).step_by(2).collect())
    } else {
        (cols.iter().collect(), cols.iter().collect())
    };
    let nc = codes.table.len();
    let a = assign(nc, &pairs(&train, Ctx::Prev), 3);
    let ap = assign(nc, &pairs(&train, Ctx::Parent), 3);
    let mut slot_codes = vec![0usize; 256];
    for &v in &a.f {
        slot_codes[v as usize] += 1;
    }
    let shared_bytes = slot_codes.iter().filter(|&&n| n > 1).count();
    let shared_codes = a.f.iter().filter(|&&v| slot_codes[v as usize] > 1).count();
    let mut r = Report {
        files: files.len(),
        nodes: cols.iter().map(|c| c.len() as u64).sum(),
        codes: nc,
        kinds,
        kind_field,
        kind_rest,
        shared_bytes,
        shared_codes,
        escapes: [0; 2],
        escapes_train: 0,
        train_nodes: train.iter().map(|c| c.len() as u64).sum(),
        measured_nodes: test.iter().map(|c| c.len() as u64).sum(),
        nodes_shared: 0,
        side_bytes: 0,
        repeat_nodes: 0,
        repeat_same: 0,
        wrong: 0,
        ns: [[0.0; 2]; 6],
        disk: [[0.0; 2]; 3],
    };
    for c in &train {
        r.escapes_train += store(c, &a, Ctx::Prev).1.len() as u64;
    }
    let mut stored: Vec<Vec<u8>> = Vec::new();
    for c in &test {
        let (bytes, esc) = store(c, &a, Ctx::Prev);
        r.escapes[0] += esc.len() as u64;
        r.escapes[1] += store(c, &ap, Ctx::Parent).1.len() as u64;
        r.nodes_shared += c.code.iter().filter(|&&k| slot_codes[a.f[k as usize] as usize] > 1).count() as u64;
        let c8 = Code8::new(bytes.clone(), &esc, &a.t);
        r.side_bytes += c8.side_bytes() as u64;
        r.wrong += (0..c.len()).filter(|&i| c8.get(i) != c.code[i] as u32).count() as u64;
        stored.push(bytes);
    }
    // Same bits over repeats, on the measured files (`occurrences` numbers them in `test`'s order).
    let test_cols: Vec<Cols> = test.iter().map(|c| Cols { code: c.code.clone(), start: c.start.clone(), end: c.end.clone(), size: c.size.clone(), parent: c.parent.clone(), source_len: c.source_len }).collect();
    for o in occurrences(&test_cols) {
        let (x, y) = (&stored[o.g], &stored[o.f]);
        r.repeat_nodes += o.size as u64 - 1;
        r.repeat_same += (1..o.size).filter(|&d| x[o.k + d] == y[o.i + d]).count() as u64;
    }
    drop(test_cols);
    // Reads on the largest measured file.
    let big = (0..test.len()).max_by_key(|&f| test[f].len()).unwrap_or(0);
    let c = test[big];
    let n = c.len();
    let (bytes, esc) = store(c, &a, Ctx::Prev);
    let c8 = Code8::new(bytes, &esc, &a.t);
    let p9 = Packed::new_width(&c.code, codes.width());
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for i in (1..order.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let best = |f: &dyn Fn() -> u64| {
        let mut b = u64::MAX;
        for _ in 0..5 {
            let (s, t) = cost(f);
            std::hint::black_box(s);
            b = b.min(t.nanos);
        }
        b as f64 / n.max(1) as f64
    };
    // Each read inlined into its own loops (a generic, not a call through a pointer).
    fn both(order: &[usize], n: usize, best: &dyn Fn(&dyn Fn() -> u64) -> f64, f: impl Fn(usize) -> u64) -> [f64; 2] {
        [
            best(&|| order.iter().fold(0u64, |s, &i| s.wrapping_add(f(i)))),
            best(&|| (0..n).fold(0u64, |s, i| s.wrapping_add(f(i)))),
        ]
    }
    r.ns = [
        both(&order, n, &best, |i| u64::from(c8.byte(i))),
        both(&order, n, &best, |i| u64::from(c8.get_fast(i))),
        both(&order, n, &best, |i| u64::from(c8.get_t16(i))),
        both(&order, n, &best, |i| u64::from(c8.get(i))),
        both(&order, n, &best, |i| p9.get(i)),
        both(&order, n, &best, |i| u64::from(c8.get_ident(i))),
    ];
    // The renumbered read agrees with the code wherever the byte is unshared.
    r.wrong += (0..n).filter(|&i| c8.one[c8.byte(i) as usize] != SHARED && c8.get_ident(i) as u16 != c8.one[c8.byte(i) as usize] && c8.get_ident(i) != u32::from(c8.byte(i))).count() as u64;
    // Every read agrees with the plain decode.
    r.wrong += (0..n).filter(|&i| c8.get_fast(i) != c8.get(i) || c8.get_t16(i) != c8.get(i)).count() as u64;
    // On disk: each form's codes array a file, with a dictionary trained on the other files' arrays.
    if split {
        let forms = |c: &Cols, a: &Assign| -> [Vec<u8>; 3] {
            let (bytes, esc) = store(c, a, Ctx::Prev);
            let mut b8 = bytes;
            // The side list rides along (offset byte and code a escape).
            for (i, code) in esc {
                b8.push((i % BLOCK) as u8);
                b8.extend_from_slice(&(code as u16).to_le_bytes());
            }
            let w = codes.width();
            let p = Packed::new_width(&c.code, w);
            let p16 = Packed::new_width(&c.code, 16);
            [b8, le_bytes(&p.words, (c.len() * w as usize).div_ceil(8)), le_bytes(&p16.words, c.len() * 2)]
        };
        let tr: Vec<[Vec<u8>; 3]> = train.iter().map(|c| forms(c, &a)).collect();
        let te: Vec<[Vec<u8>; 3]> = test.iter().map(|c| forms(c, &a)).collect();
        for k in 0..3 {
            let samples: Vec<Vec<u8>> = tr.iter().map(|x| x[k].clone()).collect();
            let d = Dictionary::train(&samples, 64 << 10);
            let mut lz = Lz4::new(true, &d.bytes);
            let (mut z, mut l) = (0u64, 0u64);
            for x in &te {
                z += codec::compress(Codec::Zstd, &x[k], &d, 0).len() as u64;
                l += lz.compress(&x[k]).len() as u64;
            }
            r.disk[k] = [z as f64 / r.measured_nodes as f64, l as f64 / r.measured_nodes as f64];
        }
    }
    r
}

pub fn print(title: &str, r: &Report) {
    let pct = |a: u64, b: u64| format!("{:.2}%", 100.0 * a as f64 / b.max(1) as f64);
    let bits = |n: usize| crate::inplace::bits_for(n.saturating_sub(1) as u64).max(1);
    println!("### {title}\n");
    println!(
        "{} files ({} nodes); {} measured nodes{}. Distinct: {} codes ({} bits), {} kinds ({} bits), {} kind + field ({} bits), {} kind + flags + supertypes ({} bits). Decoded wrong: {}.\n",
        r.files, r.nodes, r.measured_nodes,
        if r.measured_nodes < r.nodes { ", the bytes chosen on the others" } else { "" },
        r.codes, bits(r.codes), r.kinds, bits(r.kinds), r.kind_field, bits(r.kind_field), r.kind_rest, bits(r.kind_rest), r.wrong
    );
    println!("| | value |");
    println!("|---|---|");
    println!("| bytes holding two codes or more; codes sharing a byte | {}; {} |", r.shared_bytes, r.shared_codes);
    println!("| nodes escaped, context the previous byte (measured; chosen on) | {} ({}) |", pct(r.escapes[0], r.measured_nodes), pct(r.escapes_train, r.train_nodes));
    println!("| nodes escaped, context the parent's byte | {} |", pct(r.escapes[1], r.measured_nodes));
    println!("| nodes whose byte another code shares (a code-set sweep confirms them) | {} |", pct(r.nodes_shared, r.measured_nodes));
    println!("| memory, B a node: 8-bit + escapes / today's {} bits | {:.3} / {:.3} |", bits(r.codes), 1.0 + r.side_bytes as f64 / r.measured_nodes.max(1) as f64, bits(r.codes) as f64 / 8.0);
    println!("| repeated subtrees' nodes stored as the same bytes (root excluded) | {} |", pct(r.repeat_same, r.repeat_nodes));
    let names = ["the byte alone", "code: one-byte table, 2-D for shared bytes", "code: every read through the 2-D table (u16, 128 KiB)", "code: every read through the 2-D table (u32, 256 KiB; first measured)"];
    for (k, name) in names.iter().enumerate() {
        println!("| read at random / in order, ns: 8-bit, {name} | {:.2} / {:.2} |", r.ns[k][0], r.ns[k][1]);
    }
    println!("| read at random / in order, ns: 8-bit, code: renumbered, the byte unless a 256-bit set marks it shared | {:.2} / {:.2} |", r.ns[5][0], r.ns[5][1]);
    println!("| read at random / in order, ns: {}-bit packed | {:.2} / {:.2} |", bits(r.codes), r.ns[4][0], r.ns[4][1]);
    println!("| on disk, B a node, zstd 3 + dict / LZ4 HC + dict: 8-bit | {:.3} / {:.3} |", r.disk[0][0], r.disk[0][1]);
    println!("| the same: {}-bit packed | {:.3} / {:.3} |", bits(r.codes), r.disk[1][0], r.disk[1][1]);
    println!("| the same: 16-bit | {:.3} / {:.3} |", r.disk[2][0], r.disk[2][1]);
    println!();
}
