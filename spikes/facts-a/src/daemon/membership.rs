//! Membership tooling for the daemon, copied from Spike B's `spikes/facts-b/src/membership.rs`
//! (Phase 2's model, `phase-2-s3-crossfile.md` § 5.2) and kept separate from P3a's live code: a
//! per-path base (each path's most common content among the paths a majority holds), cluster
//! bases for runs of similar worktrees (an exact dynamic program over contiguous runs in name
//! order), and worktree deltas relative to their cluster or the base. Records are
//! `(tag, (path, content))` with signed multiplicity: tag 0 holds the base, a cluster tag the
//! cluster base minus the base, a worktree tag the worktree minus its parent. `Resident`
//! reference-counts the (path, content) pairs some tag's records hold, which is what the daemon
//! keeps evaluated, pinned and resident; a pair no tag holds is released.
//!
//! Differences from Spike B: no dataflow tag registry (the daemon's tags are its own), and the
//! weight of a delta entry is the caller's (the daemon weighs every entry 1: per-file results
//! cost the same whatever the content).
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

pub type W = u32;
pub type Sym = u32;
pub type Cid = u32;
pub type Members = BTreeMap<Sym, Cid>;

pub const BASE: W = 0;
pub const CLUSTER_TAG0: W = 100_000;

/// Per-path plurality over `ms`: every path held by a strict majority, with its most frequent
/// content (ties: the smallest content id).
pub fn plurality(ms: &[&Members]) -> Members {
    let mut counts: BTreeMap<Sym, BTreeMap<Cid, usize>> = BTreeMap::new();
    for m in ms {
        for (p, c) in m.iter() {
            *counts.entry(*p).or_default().entry(*c).or_default() += 1;
        }
    }
    let need = ms.len() / 2 + 1;
    counts
        .into_iter()
        .filter(|(_, hs)| hs.values().sum::<usize>() >= need)
        .map(|(p, hs)| {
            let best = hs.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).unwrap();
            (p, *best.0)
        })
        .collect()
}

/// Signed difference `m − below` as (path, content, ±1) entries.
pub fn delta(m: &Members, below: &Members) -> Vec<((Sym, Cid), i32)> {
    let mut v = Vec::new();
    for (p, c) in m {
        if below.get(p) != Some(c) {
            v.push(((*p, *c), 1));
        }
    }
    for (p, c) in below {
        if m.get(p) != Some(c) {
            v.push(((*p, *c), -1));
        }
    }
    v
}

fn cost(d: &[((Sym, Cid), i32)], weight: &dyn Fn(Sym, Cid) -> u64) -> u64 {
    d.iter().map(|((p, c), _)| weight(*p, *c)).sum()
}

pub struct Model {
    pub base: Members,
    /// cluster tag -> (worktrees, cluster base)
    pub clusters: BTreeMap<W, (Vec<W>, Members)>,
    pub parent: BTreeMap<W, W>,
    /// The cost the segmentation minimised, and the flat (no cluster) cost, for the report.
    pub cost: u64,
    pub flat_cost: u64,
}

impl Default for Model {
    fn default() -> Self {
        Model { base: Members::new(), clusters: BTreeMap::new(), parent: BTreeMap::new(), cost: 0, flat_cost: 0 }
    }
}

/// Choose cluster bases over the worktrees in order (`order`, by name): an exact dynamic program
/// over contiguous segments; a segment either becomes a cluster (its plurality base, a delta from
/// the base, plus each member's delta from it) or leaves its members directly under the base,
/// whichever weighs less.
pub fn model(order: &[(W, &Members)], weight: &dyn Fn(Sym, Cid) -> u64) -> Model {
    let all: Vec<&Members> = order.iter().map(|(_, m)| *m).collect();
    let base = plurality(&all);
    let n = order.len();
    let direct: Vec<u64> = order.iter().map(|(_, m)| cost(&delta(m, &base), weight)).collect();
    let flat_cost: u64 = direct.iter().sum();
    let mut best = vec![u64::MAX; n + 1];
    let mut choice = vec![(0usize, false); n + 1];
    best[0] = 0;
    for j in 1..=n {
        best[j] = best[j - 1] + direct[j - 1];
        choice[j] = (j - 1, false);
        for i in 0..j - 1 {
            let seg: Vec<&Members> = order[i..j].iter().map(|(_, m)| *m).collect();
            let cb = plurality(&seg);
            let c = cost(&delta(&cb, &base), weight) + seg.iter().map(|m| cost(&delta(m, &cb), weight)).sum::<u64>();
            if best[i] + c < best[j] {
                best[j] = best[i] + c;
                choice[j] = (i, true);
            }
        }
    }
    let mut segs = Vec::new();
    let mut j = n;
    while j > 0 {
        let (i, cl) = choice[j];
        segs.push((i, j, cl));
        j = i;
    }
    segs.reverse();
    let mut clusters = BTreeMap::new();
    let mut parent = BTreeMap::new();
    let mut next = CLUSTER_TAG0;
    for (i, j, cl) in segs {
        if cl {
            let seg: Vec<&Members> = order[i..j].iter().map(|(_, m)| *m).collect();
            let cb = plurality(&seg);
            let ws: Vec<W> = order[i..j].iter().map(|(w, _)| *w).collect();
            for w in &ws {
                parent.insert(*w, next);
            }
            parent.insert(next, BASE);
            clusters.insert(next, (ws, cb));
            next += 1;
        } else {
            for (w, _) in &order[i..j] {
                parent.insert(*w, BASE);
            }
        }
    }
    Model { base, clusters, parent, cost: best[n], flat_cost }
}

impl Model {
    /// The membership a tag's delta is relative to.
    pub fn below(&self, t: W) -> &Members {
        match self.parent.get(&t) {
            Some(p) if *p != BASE => &self.clusters[p].1,
            _ => &self.base,
        }
    }

    /// The level records: base, then each cluster's delta from the base.
    pub fn level_records(&self) -> Vec<(W, Vec<((Sym, Cid), i32)>)> {
        let mut v = vec![(BASE, self.base.iter().map(|(p, c)| ((*p, *c), 1)).collect())];
        for (t, (_, m)) in &self.clusters {
            v.push((*t, delta(m, &self.base)));
        }
        v
    }

    /// Attach a new worktree (a clone, or any membership) under the level it differs least from.
    pub fn attach(&mut self, w: W, m: &Members, weight: &dyn Fn(Sym, Cid) -> u64) -> W {
        let mut best = (cost(&delta(m, &self.base), weight), BASE);
        for (t, (_, cb)) in &self.clusters {
            let c = cost(&delta(m, cb), weight);
            if c < best.0 {
                best = (c, *t);
            }
        }
        self.parent.insert(w, best.1);
        best.1
    }
}

/// Reference counts of (path, content) pairs held by some tag's records: the resident set.
#[derive(Default)]
pub struct Resident {
    per: FxHashMap<(W, Sym, Cid), i32>,
    held: FxHashMap<(Sym, Cid), u32>,
}

impl Resident {
    /// Apply a record update; returns (path, content) pairs that became held / released.
    pub fn update(&mut self, w: W, p: Sym, c: Cid, d: i32, acquired: &mut Vec<(Sym, Cid)>, released: &mut Vec<(Sym, Cid)>) {
        let e = self.per.entry((w, p, c)).or_default();
        let before = *e != 0;
        *e += d;
        let after = *e != 0;
        if *e == 0 {
            self.per.remove(&(w, p, c));
        }
        if before == after {
            return;
        }
        let h = self.held.entry((p, c)).or_default();
        if after {
            *h += 1;
            if *h == 1 {
                acquired.push((p, c));
            }
        } else {
            *h -= 1;
            if *h == 0 {
                self.held.remove(&(p, c));
                released.push((p, c));
            }
        }
    }

    pub fn held(&self) -> usize {
        self.held.len()
    }

    /// Records held per tag (the per-worktree state the daemon keeps, by tag).
    pub fn records(&self) -> usize {
        self.per.len()
    }

    pub fn is_held(&self, p: Sym, c: Cid) -> bool {
        self.held.contains_key(&(p, c))
    }
}
