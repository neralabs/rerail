//! E16 (t39 step 8): relative forms read in place, priced for bytes, reads and same bits.
//!
//! Two follow-ups to E15 (`repeat.rs`), both at one width a language with escapes that name nothing (`Esc`):
//!
//! - **The leaf slot by sibling.** XF's leaf slot holds the back distance from the next node to that node's
//!   parent, which grows along a flat array (JSON's escapes). The node after a leaf always has a previous
//!   sibling `a`, and the two share a parent. So the leaf slot can hold `a`'s size instead, which stays small,
//!   and a parent read steps from sibling to sibling. Every K-th sibling holds the parent distance itself, so
//!   a read takes at most K − 1 steps. Slot tags: an inner node `size << 1`; a leaf `size(a) << 2 | 1`, or
//!   `parent distance << 2 | 3`.
//! - **Positions by gap.** Each start as its gap to the previous start (in preorder starts never decrease), each
//!   end as a length, and the absolute start of every K-th node in a side array. A start is the checkpoint
//!   plus at most K − 1 gaps; in order, a running sum. Gaps and lengths are the same in every copy of the same
//!   text, so a repeated subtree's positions are the same bits; the checkpoints sit apart.

use crate::codec::{self, Codec, Dictionary};
use crate::inplace::{Cols, Codes, Image, PFor, Packed};
use crate::measure::cost;
use crate::repeat::{Esc, esc_width, occurrences, xf_slots};
use crate::tree::{NONE, Tree};

pub const SIB_K: [usize; 3] = [4, 8, 16];
pub const POS_K: [usize; 3] = [8, 16, 32];

/// The leaf slot by sibling, every K-th sibling holding the parent distance.
pub fn sib_slots(c: &Cols, k: usize) -> Vec<u64> {
    let n = c.len();
    // Each node's previous sibling and its index among its parent's children.
    let (mut prev, mut idx) = (vec![NONE; n], vec![0u32; n]);
    let mut last = vec![NONE; n];
    for x in 1..n {
        let p = c.parent[x];
        if p == NONE {
            continue;
        }
        let l = last[p as usize];
        if l != NONE {
            prev[x] = l;
            idx[x] = idx[l as usize] + 1;
        }
        last[p as usize] = x as u32;
    }
    (0..n).map(|i| {
        if c.size[i] > 1 {
            u64::from(c.size[i]) << 1
        } else if i + 1 < n && c.parent[i + 1] != NONE {
            let j = i + 1;
            let a = prev[j];
            assert!(a != NONE, "the node after a leaf has a previous sibling");
            if idx[j] as usize % k == 0 {
                (((j - c.parent[j] as usize) as u64) << 2) | 3
            } else {
                (((j - a as usize) as u64) << 2) | 1
            }
        } else {
            1
        }
    }).collect()
}

/// XF's parent from its slot array (tag 0: one back; tag 1: the stored distance).
#[inline(always)]
fn xf_parent(s: &Esc, i: usize) -> usize {
    let v = s.get(i - 1);
    if v & 1 == 0 { i - 1 } else { i - (v >> 1) as usize }
}

/// The sibling form's parent, and the steps it took.
#[inline(always)]
fn sib_parent(s: &Esc, mut j: usize) -> (usize, u32) {
    let mut steps = 0;
    loop {
        let v = s.get(j - 1);
        if v & 1 == 0 {
            return (j - 1, steps);
        }
        if v & 2 != 0 {
            return (j - (v >> 2) as usize, steps);
        }
        assert!(v >> 2 != 0, "a sibling step moves back");
        j -= (v >> 2) as usize;
        steps += 1;
    }
}

/// Positions by gap: gaps and lengths at the language's widths, a start every `k` nodes.
pub struct Gap<'a> {
    gap: &'a Esc,
    len: &'a Esc,
    ckpt: Packed,
    /// K as a shift (K is a power of two), and the words K − 1 gaps take: constants generated code would hold.
    shift: u32,
    chunks: usize,
}

impl Gap<'_> {
    #[inline(always)]
    pub fn start(&self, i: usize) -> u64 {
        let c = i >> self.shift;
        self.ckpt.get(c) + self.gap.sum((c << self.shift) + 1, i + 1, self.chunks)
    }
    #[inline(always)]
    pub fn span(&self, i: usize) -> (u64, u64) {
        let s = self.start(i);
        (s, s + self.len.get(i))
    }
    pub fn bytes(&self) -> usize {
        self.gap.bytes() + self.len.bytes() + self.ckpt.bytes()
    }
    fn image(&self, out: &mut Vec<u8>) {
        self.gap.image(out);
        self.len.image(out);
        self.ckpt.image(out);
    }
}

/// Each node's gap to the previous start (0 for the root), and its length.
pub fn gaps(c: &Cols) -> (Vec<u64>, Vec<u64>) {
    let n = c.len();
    let gap = (0..n).map(|i| if i == 0 { 0 } else { c.start[i] - c.start[i - 1] }).collect();
    let len = (0..n).map(|i| c.end[i] - c.start[i]).collect();
    (gap, len)
}

#[derive(Default, Clone, Copy)]
pub struct Same {
    pub within: [u64; 2],
    pub across: [u64; 2],
}

/// Forms: slots XF, sibling K = 4, 8, 16; positions FOR + length, gap + length (K = 8, 16, 32 share the
/// gap and length arrays).
pub struct Report {
    pub files: usize,
    pub nodes: u64,
    pub repeated: [u64; 2],
    pub slot_bytes: [u64; 4],
    pub slot_escapes: [u64; 4],
    pub slot_same: [Same; 4],
    pub parent_wrong: [u64; 4],
    pub steps: [f64; 4],
    pub parent_ns: [f64; 4],
    pub pos_bytes: [u64; 4],
    pub pos_escapes: u64,
    /// Repeats whose gaps and lengths are equal values (so the same bits in the gap form); FOR's fields.
    pub pos_same: [Same; 2],
    pub pos_wrong: [u64; 4],
    pub span_ns: [[f64; 3]; 4],
    pub disk: [u64; 2],
    pub dict: [u64; 2],
    pub dict_nodes: u64,
    pub widths: [u32; 6],
}

pub fn run(trees: &[Tree], codes: &Codes) -> Report {
    let cols: Vec<Cols> = trees.iter().map(|t| Cols::of_tree(t, codes)).collect();
    let n_all: u64 = cols.iter().map(|c| c.len() as u64).sum();
    // Slots: XF, then sibling K; each at its language width.
    let slot_vals: Vec<Vec<Vec<u64>>> = cols.iter().map(|c| {
        let mut v = vec![xf_slots(c)];
        v.extend(SIB_K.iter().map(|&k| sib_slots(c, k)));
        v
    }).collect();
    let mut widths = [0u32; 6];
    for f in 0..4 {
        widths[f] = esc_width(slot_vals.iter().flat_map(|s| s[f].iter().copied()), 1.0);
    }
    let gl: Vec<(Vec<u64>, Vec<u64>)> = cols.iter().map(gaps).collect();
    widths[4] = std::env::var("E16_GAPW").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| esc_width(gl.iter().flat_map(|g| g.0.iter().copied()), 1.0));
    widths[5] = esc_width(gl.iter().flat_map(|g| g.1.iter().copied()), 1.0);
    let slots: Vec<Vec<Esc>> = slot_vals.iter().map(|s| (0..4).map(|f| Esc::new(&s[f], widths[f])).collect()).collect();
    let gesc: Vec<(Esc, Esc)> = gl.iter().map(|(g, l)| (Esc::new(g, widths[4]), Esc::new(l, widths[5]))).collect();
    let fors: Vec<(PFor, PFor)> = cols.iter().zip(&gl).map(|(c, (_, l))| (PFor::new(&c.start, true), PFor::new(l, false))).collect();
    let gap_of = |f: usize, k: usize| {
        let starts: Vec<u64> = cols[f].start.iter().step_by(k).copied().collect();
        assert!(k.is_power_of_two());
        Gap { gap: &gesc[f].0, len: &gesc[f].1, ckpt: Packed::new(&starts), shift: k.trailing_zeros(), chunks: (k - 1).div_ceil(gesc[f].0.per()) }
    };
    let mut r = Report {
        files: trees.len(),
        nodes: n_all,
        repeated: [0; 2],
        slot_bytes: [0; 4],
        slot_escapes: [0; 4],
        slot_same: [Same::default(); 4],
        parent_wrong: [0; 4],
        steps: [0.0; 4],
        parent_ns: [0.0; 4],
        pos_bytes: [0; 4],
        pos_escapes: 0,
        pos_same: [Same::default(); 2],
        pos_wrong: [0; 4],
        span_ns: [[0.0; 3]; 4],
        disk: [0; 2],
        dict: [0; 2],
        dict_nodes: 0,
        widths,
    };
    // Bytes, and every answer checked.
    let mut steps = [0u64; 4];
    for (f, c) in cols.iter().enumerate() {
        let n = c.len();
        for s in 0..4 {
            r.slot_bytes[s] += slots[f][s].bytes() as u64;
            r.slot_escapes[s] += slots[f][s].escapes() as u64;
        }
        for i in 1..n {
            let want = c.parent[i] as usize;
            if c.parent[i] == NONE {
                continue;
            }
            r.parent_wrong[0] += u64::from(xf_parent(&slots[f][0], i) != want);
            for s in 1..4 {
                let (p, st) = sib_parent(&slots[f][s], i);
                r.parent_wrong[s] += u64::from(p != want);
                steps[s] += u64::from(st);
            }
        }
        let (fs, fl) = &fors[f];
        r.pos_bytes[0] += (fs.bytes() + fl.bytes()) as u64;
        r.pos_escapes += (gesc[f].0.escapes() + gesc[f].1.escapes()) as u64;
        for i in 0..n {
            let s = fs.get(i);
            r.pos_wrong[0] += u64::from((s, s + fl.get(i)) != (c.start[i], c.end[i]));
        }
        for (q, &k) in POS_K.iter().enumerate() {
            let g = gap_of(f, k);
            r.pos_bytes[q + 1] += g.bytes() as u64;
            for i in 0..n {
                r.pos_wrong[q + 1] += u64::from(g.span(i) != (c.start[i], c.end[i]));
            }
        }
    }
    for s in 1..4 {
        r.steps[s] = steps[s] as f64 / n_all as f64;
    }
    // Same bits over the repeats (a subtree's last slot, and its first gap, name nodes outside it).
    for o in occurrences(&cols) {
        let w = usize::from(o.g != o.f);
        r.repeated[w] += o.size as u64;
        let add = |s: &mut Same, ok: bool| {
            if ok {
                if w == 0 { s.within[0] += o.size as u64 } else { s.across[0] += o.size as u64 }
            }
        };
        for s in 0..4 {
            let (a, b) = (&slots[o.g][s], &slots[o.f][s]);
            add(&mut r.slot_same[s], (0..o.size - 1).all(|d| a.field(o.k + d) == b.field(o.i + d)));
        }
        let (a, b) = (&gesc[o.g], &gesc[o.f]);
        add(&mut r.pos_same[1], (1..o.size).all(|d| a.0.field(o.k + d) == b.0.field(o.i + d)) && (0..o.size).all(|d| a.1.field(o.k + d) == b.1.field(o.i + d)));
        let (a, b) = (&fors[o.g], &fors[o.f]);
        add(&mut r.pos_same[0], (0..o.size).all(|d| a.0.field(o.k + d) == b.0.field(o.i + d) && a.1.field(o.k + d) == b.1.field(o.i + d)));
    }
    // On disk: FOR + length, and gap + length with a start every 16.
    let none = Dictionary::none();
    let z = |d: &[u8], dict: &Dictionary| codec::compress(Codec::Zstd, d, dict, 0).len() as u64;
    let images: Vec<[Vec<u8>; 2]> = (0..cols.len()).map(|f| {
        let mut a = Vec::new();
        fors[f].0.image(&mut a);
        fors[f].1.image(&mut a);
        let mut b = Vec::new();
        gap_of(f, 16).image(&mut b);
        [a, b]
    }).collect();
    for im in &images {
        for q in 0..2 {
            r.disk[q] += z(&im[q], &none);
        }
    }
    if cols.len() >= 16 {
        for q in 0..2 {
            let train: Vec<Vec<u8>> = images.iter().step_by(2).map(|im| im[q].clone()).collect();
            let d = Dictionary::train(&train, 112 << 10);
            r.dict[q] = images.iter().skip(1).step_by(2).map(|im| z(&im[q], &d)).sum();
        }
        r.dict_nodes = cols.iter().skip(1).step_by(2).map(|c| c.len() as u64).sum();
    }
    // Reads on the largest file: parent at random; span at random and in order.
    let big = (0..cols.len()).max_by_key(|&f| cols[f].len()).unwrap_or(0);
    let n = cols[big].len();
    let mut order: Vec<usize> = (1..n).collect();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for i in (1..order.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let best = |f: &dyn Fn() -> u64| {
        let (mut b, mut ins) = (u64::MAX, 0u64);
        for _ in 0..5 {
            let (s, c) = cost(f);
            std::hint::black_box(s);
            if c.nanos < b {
                (b, ins) = (c.nanos, c.instructions);
            }
        }
        if std::env::var_os("E16_INS").is_some() {
            eprintln!("{:.1} ns, {:.0} instructions a read", b as f64 / n.max(1) as f64, ins as f64 / n.max(1) as f64);
        }
        b as f64 / n.max(1) as f64
    };
    let sl = &slots[big];
    r.parent_ns[0] = best(&|| order.iter().fold(0u64, |s, &i| s.wrapping_add(xf_parent(&sl[0], i) as u64)));
    for s in 1..4 {
        r.parent_ns[s] = best(&|| order.iter().fold(0u64, |a, &i| a.wrapping_add(sib_parent(&sl[s], i).0 as u64)));
    }
    let (fs, fl) = &fors[big];
    r.span_ns[0] = [
        best(&|| order.iter().fold(0u64, |a, &i| a.wrapping_add(fs.get(i) ^ fl.get(i)))),
        best(&|| (0..n).fold(0u64, |a, i| a.wrapping_add(fs.get(i) ^ fl.get(i)))),
        best(&|| (0..n).fold(0u64, |a, i| a.wrapping_add(fs.get(i) ^ fl.get(i)))),
    ];
    for (q, &k) in POS_K.iter().enumerate() {
        let g = gap_of(big, k);
        let (ge, le) = (&gesc[big].0, &gesc[big].1);
        r.span_ns[q + 1] = [
            best(&|| order.iter().fold(0u64, |a, &i| {
                let (s, e) = g.span(i);
                a.wrapping_add(s ^ e)
            })),
            // In order: a running sum of the gaps.
            best(&|| {
                let mut s = 0u64;
                (0..n).fold(0u64, |a, i| {
                    s += ge.get(i);
                    a.wrapping_add(s ^ (s + le.get(i)))
                })
            }),
            // Point reads, in order (the cache warm).
            best(&|| (0..n).fold(0u64, |a, i| {
                let (s, e) = g.span(i);
                a.wrapping_add(s ^ e)
            })),
        ];
    }
    r
}

pub fn print(path: &str, r: &Report) {
    let n = r.nodes as f64;
    let pct = |a: u64, b: u64| format!("{:.1}%", 100.0 * a as f64 / b.max(1) as f64);
    println!("{path}: {} files, {} nodes; repeated subtrees: {} of nodes within a file, {} across files\n", r.files, r.nodes, pct(r.repeated[0], r.nodes), pct(r.repeated[1], r.nodes));
    println!("| slots, one width a language | XF (parent distance) | sibling, K = 4 | K = 8 | K = 16 |");
    println!("|---|---|---|---|---|");
    let row = |name: &str, f: &dyn Fn(usize) -> String| println!("| {name} | {} |", (0..4).map(f).collect::<Vec<_>>().join(" | "));
    row("width, bits", &|s| r.widths[s].to_string());
    row("B a node", &|s| format!("{:.3}", r.slot_bytes[s] as f64 / n));
    row("escaped", &|s| pct(r.slot_escapes[s], r.nodes));
    row("repeats as the same bits, within / across", &|s| format!("{} / {}", pct(r.slot_same[s].within[0], r.repeated[0]), pct(r.slot_same[s].across[0], r.repeated[1])));
    row("parent: wrong", &|s| r.parent_wrong[s].to_string());
    row("parent: sibling steps a node", &|s| if s == 0 { String::new() } else { format!("{:.2}", r.steps[s]) });
    row("parent at random, ns", &|s| format!("{:.2}", r.parent_ns[s]));
    println!();
    println!("| positions | frame of reference + length (now) | gap + length, a start every 8 | every 16 | every 32 |");
    println!("|---|---|---|---|---|");
    row("B a node", &|q| format!("{:.3}", r.pos_bytes[q] as f64 / n));
    row("wrong", &|q| r.pos_wrong[q].to_string());
    row("repeats as the same bits, within / across", &|q| {
        let s = &r.pos_same[q.min(1)];
        format!("{} / {}", pct(s.within[0], r.repeated[0]), pct(s.across[0], r.repeated[1]))
    });
    row("span at random, ns", &|q| format!("{:.2}", r.span_ns[q][0]));
    row("span in order, ns", &|q| format!("{:.2}", r.span_ns[q][1]));
    row("span, a point read each, in order, ns", &|q| format!("{:.2}", r.span_ns[q][2]));
    println!();
    println!("Gap and length widths {} and {} bits, {} escaped. On disk (zstd level 3), frame of reference + length against gap + length every 16: alone {:.3} / {:.3} B a node{}.",
        r.widths[4], r.widths[5], pct(r.pos_escapes, 2 * r.nodes), r.disk[0] as f64 / n, r.disk[1] as f64 / n,
        if r.dict_nodes > 0 { format!("; with a 112 KB dictionary (odd files) {:.3} / {:.3}", r.dict[0] as f64 / r.dict_nodes as f64, r.dict[1] as f64 / r.dict_nodes as f64) } else { String::new() });
}

/// One span read, kept out of line so its code can be read (`objdump`).
#[inline(never)]
pub fn probe_span(g: &Gap, i: usize) -> (u64, u64) {
    g.span(i)
}
