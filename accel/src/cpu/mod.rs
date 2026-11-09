//! The CPU lane, and the reference meaning of every kernel: each other lane's output must equal this one's.
//!
//! These are the dense forms the spikes measured on the cores (`tools/matrix-offload/cpu`): a counting table over
//! 16-bit codes instead of a hash join, a direct histogram, bit rows over the condensed graph for closure, AND and
//! popcount for overlaps. Work is split over partitions or rows on the rayon pool.

use crate::kernel::{Bits, Kernel, Output};
use rayon::prelude::*;

/// Runs `k` on the cores.
pub fn run(k: &Kernel) -> Output {
    match k {
        Kernel::BitProduct { a, b, min } => bit_product(a, b, min.as_deref()),
        Kernel::CountTable { loff, left, roff, right, emit } => count_table(loff, left, roff, right, *emit),
        Kernel::Histogram { off, codes, width } => histogram(off, codes, *width),
        Kernel::Closure { n, off, dst } => Output::Bits(closure(*n, off, dst)),
    }
}

/// `popcount(a[i] & b[j])` for every pair, or the pairs reaching `min[j]`.
pub fn bit_product(a: &Bits, b: &Bits, min: Option<&[u32]>) -> Output {
    assert_eq!(a.words, b.words, "bit product: rows of different widths");
    let count = |i: usize, j: usize| -> u32 { a.row(i).iter().zip(b.row(j)).map(|(x, y)| (x & y).count_ones()).sum() };
    match min {
        None => {
            let mut data = vec![0u32; a.rows * b.rows];
            data.par_chunks_mut(b.rows.max(1)).enumerate().for_each(|(i, out)| {
                for (j, o) in out.iter_mut().enumerate() {
                    *o = count(i, j);
                }
            });
            Output::Counts { rows: a.rows, cols: b.rows, data }
        }
        Some(min) => {
            assert_eq!(min.len(), b.rows, "bit product: one threshold per row of b");
            let per: Vec<Vec<(u32, u32)>> = (0..a.rows)
                .into_par_iter()
                .map(|i| (0..b.rows).filter(|&j| count(i, j) >= min[j]).map(|j| (i as u32, j as u32)).collect())
                .collect();
            Output::Pairs(per.concat())
        }
    }
}

fn parts(off: &[u32]) -> usize {
    off.len().saturating_sub(1)
}

/// Per left row, the right rows of its partition with its code; or the pairs.
pub fn count_table(loff: &[u32], left: &[u16], roff: &[u32], right: &[u16], emit: bool) -> Output {
    assert_eq!(parts(loff), parts(roff), "count table: the sides have different partitions");
    let n = parts(loff);
    if !emit {
        let mut out = vec![0u32; left.len()];
        // Partitions own disjoint ranges of `out`: split it by the left offsets.
        let mut slices: Vec<&mut [u32]> = Vec::with_capacity(n);
        let mut rest = &mut out[..];
        for p in 0..n {
            let (a, b) = rest.split_at_mut((loff[p + 1] - loff[p]) as usize);
            slices.push(a);
            rest = b;
        }
        slices.into_par_iter().enumerate().for_each_init(
            || vec![0u32; 1 << 16],
            |table, (p, out)| {
                let r = &right[roff[p] as usize..roff[p + 1] as usize];
                for &c in r {
                    table[c as usize] += 1;
                }
                for (o, &c) in out.iter_mut().zip(&left[loff[p] as usize..loff[p + 1] as usize]) {
                    *o = table[c as usize];
                }
                for &c in r {
                    table[c as usize] = 0;
                }
            },
        );
        return Output::PerRow(out);
    }
    let per: Vec<Vec<(u32, u32)>> = (0..n)
        .into_par_iter()
        .map(|p| {
            // The partition's right rows sorted by code, stably: equal codes keep the rows' order.
            let mut order: Vec<u32> = (roff[p]..roff[p + 1]).collect();
            order.sort_by_key(|&k| right[k as usize]);
            let keys: Vec<u16> = order.iter().map(|&k| right[k as usize]).collect();
            let mut out = Vec::new();
            for li in loff[p]..loff[p + 1] {
                let c = left[li as usize];
                let (lo, hi) = (keys.partition_point(|&x| x < c), keys.partition_point(|&x| x <= c));
                out.extend(order[lo..hi].iter().map(|&ri| (li, ri)));
            }
            out
        })
        .collect();
    Output::Pairs(per.concat())
}

/// Counts of each code below `width`, per partition.
pub fn histogram(off: &[u32], codes: &[u16], width: u32) -> Output {
    let (n, w) = (parts(off), width as usize);
    let mut data = vec![0u32; n * w];
    data.par_chunks_mut(w.max(1)).enumerate().for_each(|(p, out)| {
        for &c in &codes[off[p] as usize..off[p + 1] as usize] {
            assert!((c as u32) < width, "histogram: code {c} is not below {width}");
            out[c as usize] += 1;
        }
    });
    Output::Counts { rows: n, cols: w, data }
}

/// The strongly connected components of a CSR graph (Tarjan's, iterative): each node's component, numbered in the
/// order Tarjan's completes them, which is a reverse topological order (a component's successors come first).
pub fn components(n: usize, off: &[u32], dst: &[u32]) -> (Vec<u32>, usize) {
    const NONE: u32 = u32::MAX;
    let (mut index, mut low, mut comp) = (vec![NONE; n], vec![0u32; n], vec![NONE; n]);
    let (mut stack, mut on) = (Vec::new(), vec![false; n]);
    let (mut next, mut ncomp) = (0u32, 0usize);
    let mut call: Vec<(u32, u32)> = Vec::new();
    for root in 0..n as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        call.push((root, off[root as usize]));
        index[root as usize] = next;
        low[root as usize] = next;
        next += 1;
        stack.push(root);
        on[root as usize] = true;
        while let Some(&mut (v, ref mut e)) = call.last_mut() {
            let vu = v as usize;
            if *e < off[vu + 1] {
                let w = dst[*e as usize];
                *e += 1;
                let wu = w as usize;
                if index[wu] == NONE {
                    index[wu] = next;
                    low[wu] = next;
                    next += 1;
                    stack.push(w);
                    on[wu] = true;
                    call.push((w, off[wu]));
                } else if on[wu] {
                    low[vu] = low[vu].min(index[wu]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent as usize] = low[parent as usize].min(low[vu]);
            }
            if low[vu] == index[vu] {
                loop {
                    let w = stack.pop().unwrap();
                    on[w as usize] = false;
                    comp[w as usize] = ncomp as u32;
                    if w == v {
                        break;
                    }
                }
                ncomp += 1;
            }
        }
    }
    (comp, ncomp)
}

/// The transitive closure: components condensed, then bit rows propagated over their DAG, successors first.
pub fn closure(n: u32, off: &[u32], dst: &[u32]) -> Bits {
    let n = n as usize;
    let words = n.div_ceil(64).max(1);
    let (comp, ncomp) = components(n, off, dst);
    // Members of each component, and whether it holds a cycle (more than one node, or a self edge).
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); ncomp];
    for v in 0..n {
        members[comp[v] as usize].push(v as u32);
    }
    let mut cyclic: Vec<bool> = members.iter().map(|m| m.len() > 1).collect();
    for v in 0..n {
        for &w in &dst[off[v] as usize..off[v + 1] as usize] {
            if w as usize == v {
                cyclic[comp[v] as usize] = true;
            }
        }
    }
    let mut reach = vec![0u64; ncomp * words];
    let mut row = vec![0u64; words];
    // Tarjan's numbering puts a component's successors before it: fill in that order.
    for c in 0..ncomp {
        row.iter_mut().for_each(|w| *w = 0);
        if cyclic[c] {
            for &v in &members[c] {
                row[v as usize / 64] |= 1 << (v % 64);
            }
        }
        for &v in &members[c] {
            for &w in &dst[off[v as usize] as usize..off[v as usize + 1] as usize] {
                let d = comp[w as usize] as usize;
                if d == c {
                    continue;
                }
                row[w as usize / 64] |= 1 << (w % 64);
                for (x, y) in row.iter_mut().zip(&reach[d * words..(d + 1) * words]) {
                    *x |= y;
                }
            }
        }
        reach[c * words..(c + 1) * words].copy_from_slice(&row);
    }
    let mut data = vec![0u64; n * words];
    data.par_chunks_mut(words).enumerate().for_each(|(v, out)| {
        let c = comp[v] as usize;
        out.copy_from_slice(&reach[c * words..(c + 1) * words]);
    });
    Bits::new(n, words, data)
}

#[cfg(test)]
mod tests;
