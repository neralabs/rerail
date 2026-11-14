//! E9 (0156's T8): the engine's base relations kept as copied rows, against kept as the tape and rebuilt.
//!
//! Inside a run the engine reads base relations off its decoded form of the tape (`facts::core::Facts`: kind,
//! parent, span, field, children in CSR, kind postings; `span_within` per looked-up node, `vtab/syntax/within.rs`).
//! At its boundaries it copies them: a memo entry stores each document's base rows, and the kept cross part
//! stores its inputs' rows as frames, `span_within` made for every node. Those copies are what this measures,
//! against keeping only the tape and rebuilding what a reader asks:
//!
//! - **Bytes.** Each relation's rows as the store encodes them (`content_store::cross_rows::encode`, copied here:
//!   a column at a time, zigzag varints of the difference from the row above), one block per document (a memo
//!   entry's) and one per corpus (a kept frame's), against the tape as stored.
//! - **Rebuild.** Rows decoded and indexed for a probe (sorted by the probed column), against the tape decoded
//!   to a tree and its reader columns derived (children in CSR, the line table).
//! - **Probes.** A join's lookups (`child` by parent, `field` by parent, `span_within` by either side, `line` by
//!   node) on the rows (binary search over the sorted column) and on the columns (CSR, a parent climb, a
//!   subtree interval).
//!
//! The rows follow the engine's counts on `impl/sqlite` (node 435,959; child 435,737 = node less the roots; line
//! 436,182; span_within 3,960,870): `node(d, n, kind, start, end)` every node, `child(p, c, i)` every non-root
//! node, `field(p, f, c)` every node in a field, `line(n, l)` every node, `span_within(inner, outer)` each node
//! with itself, its ancestors and its descendants of the same span. A zero-width node's extra containers
//! (`within.rs`) are left out.

use crate::tree::{NONE, Tree};

/// `cross_rows::encode`: rows of `width`, flat, as a block.
pub fn encode(width: usize, values: &[i64], out: &mut Vec<u8>) {
    for c in 0..width {
        let mut last = 0i64;
        for row in values.chunks_exact(width) {
            let d = row[c].wrapping_sub(last);
            last = row[c];
            let mut z = ((d << 1) ^ (d >> 63)) as u64;
            while z >= 0x80 {
                out.push((z as u8) | 0x80);
                z >>= 7;
            }
            out.push(z as u8);
        }
    }
}

/// `cross_rows::decode`: a block's columns.
pub fn decode(width: usize, rows: usize, data: &[u8], cols: &mut [Vec<i64>]) {
    let mut at = 0;
    for col in cols.iter_mut().take(width) {
        col.reserve(rows);
        let mut last = 0i64;
        for _ in 0..rows {
            let (mut z, mut shift) = (0u64, 0);
            loop {
                let b = data[at];
                at += 1;
                z |= u64::from(b & 0x7f) << shift;
                if b < 0x80 {
                    break;
                }
                shift += 7;
            }
            last = last.wrapping_add(((z >> 1) as i64) ^ -((z & 1) as i64));
            col.push(last);
        }
    }
}

pub const NAMES: [&str; 5] = ["node", "child", "field", "line", "span_within"];
pub const WIDTHS: [usize; 5] = [5, 3, 3, 2, 2];

/// One document's base relations, flat rows, ids from `base`, sorted by their first column (as a frame holds
/// them; `child` by parent then position).
pub fn rows(tree: &Tree, doc: i64, base: i64, line_starts: &[u64]) -> [Vec<i64>; 5] {
    let n = tree.len();
    let mut node = Vec::with_capacity(n * 5);
    let mut field = Vec::new();
    let mut line = Vec::with_capacity(n * 2);
    let mut child: Vec<[i64; 3]> = Vec::with_capacity(n);
    let mut index = vec![0i64; n];
    let mut within = Vec::new();
    for (i, x) in tree.nodes.iter().enumerate() {
        let id = base + i as i64;
        node.extend([doc, id, i64::from(x.kind), x.start as i64, x.end as i64]);
        line.extend([id, line_of(line_starts, x.start) as i64]);
        let p = tree.parent[i];
        if p != NONE {
            child.push([base + p as i64, id, index[p as usize]]);
            index[p as usize] += 1;
            if x.field != 0 {
                field.extend([base + p as i64, i64::from(x.field), id]);
            }
        }
        // Itself and its ancestors; then its descendants of the same span (a chain of first children).
        let mut a = i as u32;
        while a != NONE {
            within.extend([id, base + a as i64]);
            a = tree.parent[a as usize];
        }
        let mut d = i + 1;
        while d < i + tree.size[i] as usize && tree.nodes[d].start == x.start && tree.nodes[d].end == x.end {
            within.extend([id, base + d as i64]);
            d += 1;
        }
    }
    child.sort_unstable();
    let mut f3: Vec<[i64; 3]> = field.chunks_exact(3).map(|r| [r[0], r[1], r[2]]).collect();
    f3.sort_unstable();
    [node, child.concat(), f3.concat(), line, within]
}

/// The 1-based line of byte `at`.
pub fn line_of(starts: &[u64], at: u64) -> usize {
    starts.partition_point(|&s| s <= at)
}

/// Each line's first byte.
pub fn line_starts(source: &[u8]) -> Vec<u64> {
    let mut out = vec![0u64];
    out.extend(source.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i as u64 + 1));
    out
}

/// A reader's columns derived from a tree (`Facts`' parts a base relation's probe reads): children in CSR over
/// parents, and the line table.
pub struct Columns {
    pub child_offsets: Vec<u32>,
    pub children: Vec<u32>,
    pub line_starts: Vec<u64>,
    /// Each node's line, by one merge of the nodes' starts (in preorder, never decreasing) with the line table.
    pub line: Vec<u32>,
}

pub fn columns(tree: &Tree, source: &[u8]) -> Columns {
    let n = tree.len();
    let mut child_offsets = vec![0u32; n + 1];
    for &p in &tree.parent {
        if p != NONE {
            child_offsets[p as usize + 1] += 1;
        }
    }
    for i in 0..n {
        child_offsets[i + 1] += child_offsets[i];
    }
    let mut at = child_offsets.clone();
    let mut children = vec![0u32; child_offsets[n] as usize];
    for (i, &p) in tree.parent.iter().enumerate() {
        if p != NONE {
            children[at[p as usize] as usize] = i as u32;
            at[p as usize] += 1;
        }
    }
    let line_starts = line_starts(source);
    let mut line = Vec::with_capacity(n);
    let mut l = 0usize;
    for x in &tree.nodes {
        while l < line_starts.len() && line_starts[l] <= x.start {
            l += 1;
        }
        line.push(l as u32);
    }
    Columns { child_offsets, children, line_starts, line }
}

/// The line table as a tape could store it (no source needed for `line`): each line's length, a varint.
pub fn line_table_bytes(starts: &[u64]) -> usize {
    starts.windows(2).map(|w| (64 - (w[1] - w[0]).max(1).leading_zeros() as usize).div_ceil(7)).sum::<usize>() + 1
}

/// The rows of column `c` (of `width`) equal to `key`, in rows sorted by that column: their range.
pub fn range(col: &[i64], key: i64) -> std::ops::Range<usize> {
    let lo = col.partition_point(|&v| v < key);
    let hi = lo + col[lo..].partition_point(|&v| v <= key);
    lo..hi
}
