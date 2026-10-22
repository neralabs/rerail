//! Base and per-worktree deltas (step 2). Base = every (path, hash) held by a strict majority of the
//! 40 worktrees. A worktree's delta is the signed difference M_w - M_B.
use crate::store::{self, Membership};
use std::collections::BTreeMap;

pub const WORKTREES: usize = 40;

pub fn worktree_names() -> Vec<String> {
    (1..=WORKTREES).map(|i| format!("{i:02}")).collect()
}

pub fn load_memberships() -> Vec<(String, Membership)> {
    let dir = store::spike_dir().join("membership");
    worktree_names().into_iter().map(|w| {
        let m = store::read_membership(&dir.join(format!("{w}.tsv")));
        (w, m)
    }).collect()
}

pub fn majority_base(ms: &[(String, Membership)]) -> Membership {
    let mut counts: BTreeMap<(&str, [u8; 32]), usize> = BTreeMap::new();
    for (_, m) in ms {
        for (p, h) in m {
            *counts.entry((p.as_str(), *h)).or_default() += 1;
        }
    }
    let need = ms.len() / 2 + 1;
    counts.into_iter().filter(|&(_, c)| c >= need).map(|((p, h), _)| (p.to_string(), h)).collect()
}

/// Per-path plurality: every path held by a strict majority of the worktrees, with its most
/// frequent content (ties: the smallest hash). Unlike `majority_base`, a path whose content churns
/// (no single content in a majority) stays in the base.
pub fn plurality_base(ms: &[(String, Membership)]) -> Membership {
    let mut counts: BTreeMap<&str, BTreeMap<[u8; 32], usize>> = BTreeMap::new();
    for (_, m) in ms {
        for (p, h) in m {
            *counts.entry(p.as_str()).or_default().entry(*h).or_default() += 1;
        }
    }
    let need = ms.len() / 2 + 1;
    counts
        .into_iter()
        .filter(|(_, hs)| hs.values().sum::<usize>() >= need)
        .map(|(p, hs)| {
            let best = hs.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).unwrap();
            (p.to_string(), *best.0)
        })
        .collect()
}

/// Signed delta: (path, hash, +1) for entries of `m` not in `base`, (path, hash, -1) for the reverse.
pub fn delta(base: &Membership, m: &Membership) -> Vec<(String, [u8; 32], i64)> {
    let b: BTreeMap<&str, [u8; 32]> = base.iter().map(|(p, h)| (p.as_str(), *h)).collect();
    let w: BTreeMap<&str, [u8; 32]> = m.iter().map(|(p, h)| (p.as_str(), *h)).collect();
    let mut out = Vec::new();
    for (p, h) in &w {
        if b.get(p) != Some(h) {
            out.push((p.to_string(), *h, 1));
        }
    }
    for (p, h) in &b {
        if w.get(p) != Some(h) {
            out.push((p.to_string(), *h, -1));
        }
    }
    out.sort();
    out
}
