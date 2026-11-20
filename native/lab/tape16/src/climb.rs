//! t39 step 23: what a parent read and a climb cost in slot reads, under each leaf-slot rule, and what each rule
//! costs in memory (`tape16 climb`). Counts, not times: a read is one slot, each a dependent load.
//!
//! The rules: the E16 sibling slot at K = 4, 8, 16 (a leaf names the parent on every K-th sibling, else its
//! previous sibling), and `fit` at K = 8 (a leaf also names the parent whenever the distance is at most a
//! limit, up to what fits the slot's width: `relative::sib_slots_fit`). XF reads one slot a parent.

use crate::inplace::{Codes, Cols};
use crate::relative::sib_slots_fit;
use crate::repeat::{Esc, Hist};
use crate::tree::{NONE, Tree};

/// (name, K, the largest parent distance a leaf names off the K-th sibling: 0 none, u64::MAX to the width)
pub const RULES: [(&str, usize, u64); 7] = [("K = 4", 4, 0), ("K = 8 (E16)", 8, 0), ("K = 16", 16, 0), ("fit to 7", 8, 7),
    ("fit to 15", 8, 15), ("fit to 31", 8, 31), ("fit to the width", 8, u64::MAX)];

/// The slot-value limit for a rule's distance at width w.
fn limit(fit: u64, w: u32) -> u64 {
    let esc = (1u64 << w) - 1;
    if fit == 0 { 0 } else if fit == u64::MAX { esc } else { ((fit << 2) | 4).min(esc) }
}

/// The parent of j from the slots, and the slots read.
fn parent(sl: &[u64], mut j: usize) -> (usize, u32) {
    let mut reads = 0;
    loop {
        let v = sl[j - 1];
        reads += 1;
        if v & 1 == 0 {
            return (j - 1, reads);
        }
        if v & 2 != 0 {
            return (j - (v >> 2) as usize, reads);
        }
        j -= (v >> 2) as usize;
    }
}

/// A language's slots under each rule: bytes a node (fixed width + escapes in parts), escaped share, and the
/// slots a point parent reads on average and at most.
pub struct Memory {
    pub nodes: usize,
    /// Per rule: bytes a node, escaped %, mean and most slots a point parent reads, zstd 3 bytes a node (a file
    /// at a time, no dictionary).
    pub rows: Vec<(f64, f64, f64, u32, f64)>,
}

pub fn memory(files: &[Cols]) -> Memory {
    let mut rows = Vec::new();
    let nodes: usize = files.iter().map(Cols::len).sum();
    for &(_, k, fit) in &RULES {
        // The width as the language learns it, from the E16 values; `fit` then fills the room under it.
        let mut h = Hist::default();
        for c in files {
            h.add(sib_slots_fit(c, k, 0).into_iter());
        }
        let w = h.width_bytes();
        let (mut bytes, mut esc, mut reads, mut parents, mut most, mut zst) = (0usize, 0usize, 0u64, 0u64, 0u32, 0usize);
        for c in files {
            let sl = sib_slots_fit(c, k, limit(fit, w));
            let e = Esc::new(&sl, w).use_parts(c.len());
            bytes += e.bytes();
            let (main, side) = e.split(c.len());
            zst += zstd::bulk::compress(&main, 3).expect("zstd").len() + zstd::bulk::compress(&side, 3).expect("zstd").len();
            esc += e.escapes();
            for j in 1..c.len() {
                if c.parent[j] == NONE {
                    continue;
                }
                let (p, r) = parent(&sl, j);
                assert_eq!(p, c.parent[j] as usize, "the slots name the parent");
                reads += u64::from(r);
                parents += 1;
                most = most.max(r);
            }
        }
        rows.push((bytes as f64 / nodes as f64, 100.0 * esc as f64 / nodes as f64, reads as f64 / parents.max(1) as f64, most, zst as f64 / nodes as f64));
    }
    Memory { nodes, rows }
}

/// One set's climbs over a forest, per 64-node group: the full climb from the group's first node to the root
/// (what `nearest` and `outermost` do now), and the bounded one (only when the group has an S-ancestor, to the
/// first S-ancestor enclosing the group, or the outermost). Per rule: slot reads a node of the forest; and
/// parent steps a node (XF's slot reads).
pub struct Climbs {
    pub full: Vec<f64>,
    pub bounded: Vec<f64>,
    pub steps: (f64, f64),
}

pub fn climbs(tree: &Tree, codes: &Codes, set: &[bool]) -> Climbs {
    let c = Cols::of_tree(tree, codes);
    let n = c.len();
    let inset: Vec<bool> = c.code.iter().map(|&k| set[k as usize]).collect();
    let end = |p: usize| p + c.size[p] as usize;
    let (mut full, mut bounded) = (Vec::new(), Vec::new());
    let mut steps = (0u64, 0u64);
    let mut h = Hist::default();
    h.add(sib_slots_fit(&c, 8, 0).into_iter());
    let w = h.width_bytes();
    for (ri, &(_, k, fit)) in RULES.iter().enumerate() {
        let sl = sib_slots_fit(&c, k, limit(fit, w));
        let (mut rf, mut rb) = (0u64, 0u64);
        for k0 in (0..n).step_by(64) {
            let ge = (k0 + 64).min(n);
            // The full climb.
            let mut j = k0;
            let mut top = NONE as usize;
            let mut sf = 0u64;
            while j != 0 && c.parent[j] != NONE {
                let (p, r) = parent(&sl, j);
                rf += u64::from(r);
                sf += 1;
                if inset[p] {
                    top = p;
                }
                j = p;
            }
            let any = top != NONE as usize;
            // The bounded climb: none without an S-ancestor; else up to one enclosing the group.
            let mut sb = 0u64;
            if any {
                let mut j = k0;
                while j != 0 && c.parent[j] != NONE {
                    let (p, r) = parent(&sl, j);
                    rb += u64::from(r);
                    sb += 1;
                    if inset[p] && (end(p) >= ge || p == top) {
                        break;
                    }
                    j = p;
                }
            }
            if ri == 0 {
                steps.0 += sf;
                steps.1 += sb;
            }
        }
        full.push(rf as f64 / n as f64);
        bounded.push(rb as f64 / n as f64);
    }
    Climbs { full, bounded, steps: (steps.0 as f64 / n as f64, steps.1 as f64 / n as f64) }
}
