//! Edits (E4) and node identity (T7): for a file's old and new revision, which blocks are new, and how many
//! per-node rows downstream change under each way of naming a node.
//!
//! Nodes are matched between revisions through a line diff of the two sources (patience: lines unique in
//! both anchor, then the gaps between anchors recursively, trimming equal ends). A new node whose span lies
//! in one unchanged region matches the old node of the same kind, field and flags at the mapped span; an
//! unmatched node whose start lies in an unchanged region matches the old node of the same kind and field at
//! the mapped start (it contains an edit: an ancestor). The rest is what the edits replaced.

use crate::blocks::{self, Cut};
use crate::events::Events;
use crate::tree::{NONE, Tree};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct EditStats {
    pub pairs: usize,
    pub old_nodes: u64,
    pub new_nodes: u64,
    /// Nodes in neither the prefix nor the suffix (replaced), and the prefix's whose end moved (ancestors).
    pub replaced: u64,
    pub ancestors: u64,
    pub blocks_total: u64,
    pub blocks_new: u64,
    pub block_bytes_new: u64,
    pub tape_bytes: u64,
    /// Rows whose key changed, per identity.
    pub key_changed: [u64; 4],
    /// Rows whose key or parent's key changed, per identity.
    pub link_changed: [u64; 4],
    /// Nodes whose absolute span moved (every identity: spans are positions).
    pub span_moved: u64,
    /// Identity D: one range shift per pair that moved nodes, covering this many rows.
    pub shift_ops: u64,
    pub shifted_rows: u64,
}

pub const IDENTITIES: [&str; 4] = ["A ordinal", "B block+local", "C path", "D ordinal+shift"];

fn line_hashes(src: &[u8]) -> (Vec<u64>, Vec<usize>) {
    let (mut h, mut starts) = (Vec::new(), vec![0usize]);
    for (i, line) in src.split_inclusive(|&b| b == b'\n').enumerate() {
        let mut x = 0xcbf29ce484222325u64;
        for &b in line {
            x = (x ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        h.push(x);
        starts.push(starts[i] + line.len());
    }
    (h, starts)
}

/// Matched line pairs (old, new), ascending: patience diff.
fn patience(a: &[u64], b: &[u64]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut work = vec![(0usize, a.len(), 0usize, b.len())];
    while let Some((mut a0, mut a1, mut b0, mut b1)) = work.pop() {
        while a0 < a1 && b0 < b1 && a[a0] == b[b0] {
            out.push((a0, b0));
            (a0, b0) = (a0 + 1, b0 + 1);
        }
        while a0 < a1 && b0 < b1 && a[a1 - 1] == b[b1 - 1] {
            out.push((a1 - 1, b1 - 1));
            (a1, b1) = (a1 - 1, b1 - 1);
        }
        if a0 == a1 || b0 == b1 {
            continue;
        }
        let mut count: HashMap<u64, (u32, u32, usize, usize)> = HashMap::new();
        for i in a0..a1 {
            let e = count.entry(a[i]).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.2 = i;
        }
        for j in b0..b1 {
            if let Some(e) = count.get_mut(&b[j]) {
                e.1 += 1;
                e.3 = j;
            }
        }
        let mut uniq: Vec<(usize, usize)> = count.values().filter(|e| e.0 == 1 && e.1 == 1).map(|e| (e.2, e.3)).collect();
        if uniq.is_empty() {
            continue;
        }
        uniq.sort_unstable();
        // The longest increasing run of new indexes (patience sorting).
        let mut tails: Vec<usize> = Vec::new();
        let mut back = vec![usize::MAX; uniq.len()];
        for (k, &(_, j)) in uniq.iter().enumerate() {
            let at = tails.partition_point(|&t| uniq[t].1 < j);
            if at > 0 {
                back[k] = tails[at - 1];
            }
            if at == tails.len() {
                tails.push(k);
            } else {
                tails[at] = k;
            }
        }
        let mut chain = Vec::new();
        let mut k = *tails.last().unwrap();
        loop {
            chain.push(uniq[k]);
            if back[k] == usize::MAX {
                break;
            }
            k = back[k];
        }
        chain.reverse();
        let (mut pa, mut pb) = (a0, b0);
        for &(i, j) in &chain {
            out.push((i, j));
            work.push((pa, i, pb, j));
            (pa, pb) = (i + 1, j + 1);
        }
        work.push((pa, a1, pb, b1));
    }
    out.sort_unstable();
    out
}

/// For each new node, its old node (or `usize::MAX`), and how many matched nodes are ancestors of an edit.
pub fn match_nodes(old: &Tree, new: &Tree, old_src: &[u8], new_src: &[u8]) -> (Vec<usize>, usize) {
    let (ha, sa) = line_hashes(old_src);
    let (hb, sb) = line_hashes(new_src);
    // Unchanged regions in byte terms: runs of consecutive matched lines, (new start, new end, old start).
    let mut regions: Vec<(u64, u64, u64)> = Vec::new();
    for (i, j) in patience(&ha, &hb) {
        let (ns, ne, os) = (sb[j] as u64, sb[j + 1] as u64, sa[i] as u64);
        match regions.last_mut() {
            Some(r) if r.1 == ns && r.2 + (r.1 - r.0) == os => r.1 = ne,
            _ => regions.push((ns, ne, os)),
        }
    }
    // The (old − new) offset of the unchanged region holding [pos, end), if one does.
    let region_of = |pos: u64, end: u64| -> Option<i64> {
        let k = regions.partition_point(|r| r.0 <= pos);
        let r = regions.get(k.checked_sub(1)?)?;
        (end <= r.1).then_some(r.2 as i64 - r.0 as i64)
    };
    let mut by_span: HashMap<(u64, u64, u16, u8, u8), usize> = HashMap::new();
    let mut by_start: HashMap<(u64, u16, u8), usize> = HashMap::new();
    for (i, n) in old.nodes.iter().enumerate() {
        by_span.entry((n.start, n.end, n.kind, n.field, n.flags)).or_insert(i);
        by_start.entry((n.start, n.kind, n.field)).or_insert(i);
    }
    let mut of_new = vec![usize::MAX; new.len()];
    let mut ancestors = 0;
    let mut used = vec![false; old.len()];
    for (j, n) in new.nodes.iter().enumerate() {
        if let Some(d) = region_of(n.start, n.end) {
            let key = ((n.start as i64 + d) as u64, (n.end as i64 + d) as u64, n.kind, n.field, n.flags);
            if let Some(&i) = by_span.get(&key) {
                if !used[i] {
                    (of_new[j], used[i]) = (i, true);
                    continue;
                }
            }
        }
        if let Some(d) = region_of(n.start, n.start) {
            if let Some(&i) = by_start.get(&((n.start as i64 + d) as u64, n.kind, n.field)) {
                if !used[i] {
                    (of_new[j], used[i]) = (i, true);
                    ancestors += 1;
                }
            }
        }
    }
    (of_new, ancestors)
}

/// Keys under C: (parent's key, kind, field, ordinal among siblings with that kind and field), hashed.
fn path_keys(t: &Tree) -> Vec<u64> {
    let mut key = vec![0u64; t.len()];
    let mut seen: HashMap<(u32, u16, u8), u32> = HashMap::new();
    for i in 0..t.len() {
        let p = t.parent[i];
        let n = &t.nodes[i];
        let ord = seen.entry((p, n.kind, n.field)).or_insert(0);
        let pk = if p == NONE { 0x51 } else { key[p as usize] };
        let mut h = pk ^ 0x9e3779b97f4a7c15;
        for v in [n.kind as u64, n.field as u64, *ord as u64] {
            h = (h ^ v).wrapping_mul(0x100000001b3).rotate_left(17);
        }
        key[i] = h;
        *ord += 1;
    }
    key
}

/// Keys under B: (block body's hash, index in block).
fn block_keys(ev: &Events, cut: Cut, lay: impl Fn(&blocks::Block) -> Vec<u8>) -> (Vec<(u64, u32)>, Vec<(u64, usize)>) {
    let (bs, _) = blocks::split(ev, cut);
    let mut keys = Vec::with_capacity(ev.opens.len());
    let mut bodies = Vec::with_capacity(bs.len());
    for b in &bs {
        let bytes = lay(b);
        let h = u64::from_le_bytes(blake3::hash(&bytes).as_bytes()[..8].try_into().unwrap());
        for k in 0..b.opens.len() {
            keys.push((h, k as u32));
        }
        bodies.push((h, bytes.len()));
    }
    (keys, bodies)
}

#[allow(clippy::too_many_arguments)]
pub fn compare(
    old: &Tree,
    new: &Tree,
    old_src: &[u8],
    new_src: &[u8],
    old_ev: &Events,
    new_ev: &Events,
    cut: Cut,
    lay: &dyn Fn(&blocks::Block) -> Vec<u8>,
    stats: &mut EditStats,
) {
    let (n0, n1) = (old.len(), new.len());
    let (of_new, ancestors) = match_nodes(old, new, old_src, new_src);
    let matched = of_new.iter().filter(|&&i| i != usize::MAX).count();
    stats.pairs += 1;
    stats.old_nodes += n0 as u64;
    stats.new_nodes += n1 as u64;
    stats.replaced += (n1 - matched) as u64;
    stats.ancestors += ancestors as u64;
    // Blocks: new bodies not among the old revision's.
    let (kb0, b0) = block_keys(old_ev, cut, lay);
    let (kb1, b1) = block_keys(new_ev, cut, lay);
    let old_bodies: HashSet<u64> = b0.iter().map(|b| b.0).collect();
    stats.blocks_total += b1.len() as u64;
    for &(h, len) in &b1 {
        if !old_bodies.contains(&h) {
            stats.blocks_new += 1;
            stats.block_bytes_new += len as u64;
        }
        stats.tape_bytes += len as u64;
    }
    let kc0 = path_keys(old);
    let kc1 = path_keys(new);
    for j in 0..n1 {
        let i = of_new[j];
        if i == usize::MAX {
            for id in 0..4 {
                stats.key_changed[id] += 1;
                stats.link_changed[id] += 1;
            }
            stats.span_moved += 1;
            continue;
        }
        if new.nodes[j].start != old.nodes[i].start || new.nodes[j].end != old.nodes[i].end {
            stats.span_moved += 1;
        }
        let key_a = i != j;
        let key_b = kb0[i] != kb1[j];
        let key_c = kc0[i] != kc1[j];
        let (pi, pj) = (old.parent[i], new.parent[j]);
        let parent_a = (pi == NONE) != (pj == NONE) || (pj != NONE && pi != pj);
        let parent_b = pj != NONE && (pi == NONE || kb0[pi as usize] != kb1[pj as usize]);
        let parent_c = pj != NONE && (pi == NONE || kc0[pi as usize] != kc1[pj as usize]);
        // D: keys are A's, but a shift map renumbers matched nodes; a row changes only if its node's parent
        // is not the shifted image of its old parent.
        let parent_d = pj != NONE && (pi == NONE || of_new[pj as usize] != pi as usize);
        stats.key_changed[0] += key_a as u64;
        stats.key_changed[1] += key_b as u64;
        stats.key_changed[2] += key_c as u64;
        stats.link_changed[0] += (key_a || parent_a) as u64;
        stats.link_changed[1] += (key_b || parent_b) as u64;
        stats.link_changed[2] += (key_c || parent_c) as u64;
        stats.link_changed[3] += parent_d as u64;
    }
    // D: one shift op per run of matched nodes with the same (new − old) ordinal offset, when it is not 0.
    let mut last: Option<i64> = None;
    for (j, &i) in of_new.iter().enumerate() {
        if i == usize::MAX {
            continue;
        }
        let d = j as i64 - i as i64;
        if d != 0 {
            stats.shifted_rows += 1;
            if last != Some(d) {
                stats.shift_ops += 1;
            }
        }
        last = Some(d);
    }
}
