//! Edits (E4) and node identity (T7): for a file's old and new revision, which blocks are new, and how many
//! per-node rows downstream change under each way of naming a node.
//!
//! Nodes are matched between revisions by the longest equal prefix of preorder (kind, field, flags,
//! classes, start) and the longest equal suffix (the same, with spans shifted by the length change). The
//! prefix's nodes whose end moved are the edit's ancestors; the rest between is what the edit replaced.

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

fn same(a: &Tree, i: usize, b: &Tree, j: usize, shift: i64) -> bool {
    let (x, y) = (&a.nodes[i], &b.nodes[j]);
    x.kind == y.kind && x.field == y.field && x.flags == y.flags && x.classes == y.classes && x.start as i64 + shift == y.start as i64
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

pub fn compare(
    old: &Tree,
    new: &Tree,
    old_ev: &Events,
    new_ev: &Events,
    cut: Cut,
    lay: &dyn Fn(&blocks::Block) -> Vec<u8>,
    stats: &mut EditStats,
) {
    let (n0, n1) = (old.len(), new.len());
    let shift = new.source_len as i64 - old.source_len as i64;
    let mut pre = 0;
    while pre < n0 && pre < n1 && same(old, pre, new, pre, 0) {
        pre += 1;
    }
    let mut suf = 0;
    while suf < n0 - pre && suf < n1 - pre && same(old, n0 - 1 - suf, new, n1 - 1 - suf, shift) && old.nodes[n0 - 1 - suf].end as i64 + shift == new.nodes[n1 - 1 - suf].end as i64 {
        suf += 1;
    }
    // The match: new node j ↔ old node.
    let mut of_new = vec![usize::MAX; n1];
    for j in 0..pre {
        of_new[j] = j;
    }
    for s in 0..suf {
        of_new[n1 - 1 - s] = n0 - 1 - s;
    }
    let ancestors = (0..pre).filter(|&j| old.nodes[j].end != new.nodes[j].end).count();
    stats.pairs += 1;
    stats.old_nodes += n0 as u64;
    stats.new_nodes += n1 as u64;
    stats.replaced += (n1 - pre - suf) as u64;
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
    let moved = n1 != n0;
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
    if moved && suf > 0 {
        stats.shift_ops += 1;
        stats.shifted_rows += suf as u64;
    }
}
