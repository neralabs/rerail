//! From-scratch oracle (step 5): R1-R3 evaluated naively over one worktree's membership joined
//! with the per-file records. No base, no deltas, no dataflow.
use crate::derive::{Node, World};
use crate::rules::{self, Family, Root, Surface, SurfaceOrIssue, R1F, R2F, R3F, MAX_HOPS};
use crate::store::{Cid, Sym};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct Findings {
    pub r1: BTreeSet<R1F>,
    pub r2: BTreeSet<R2F>,
    pub r3: BTreeSet<R3F>,
}

pub fn evaluate(w: &mut World, files: &[(Sym, Cid)]) -> Findings {
    Findings { r1: r1(w, files), r2: r2(w, files), r3: r3(w, files) }
}

fn r1(w: &World, files: &[(Sym, Cid)]) -> BTreeSet<R1F> {
    let mut referenced: BTreeSet<Sym> = BTreeSet::new();
    for &(_, c) in files {
        referenced.extend(w.r1_refs(c));
    }
    let mut out = BTreeSet::new();
    for &(p, c) in files {
        for (name, p, q) in w.r1_cand(p, c) {
            if !referenced.contains(&name) {
                out.insert((p, q));
            }
        }
    }
    out
}

fn r2(w: &mut World, files: &[(Sym, Cid)]) -> BTreeSet<R2F> {
    let mut decl: BTreeMap<(Sym, Sym), BTreeSet<(Sym, u8)>> = BTreeMap::new();
    let mut bind: BTreeMap<(Sym, Sym), BTreeSet<(Sym, Sym)>> = BTreeMap::new();
    let mut exports: Vec<((Sym, Sym), (Sym, Sym))> = Vec::new();
    for &(p, c) in files {
        for (k, v) in w.r2_decl(p, c) {
            decl.entry(k).or_default().insert(v);
        }
        for (k, v) in w.r2_bind(p, c) {
            bind.entry(k).or_default().insert(v);
        }
        exports.extend(w.r2_export(p, c));
    }
    // Resolve each root through at most MAX_HOPS re-export hops; a hop is followed only where no
    // declaration has the target's (module, name).
    let mut roots: BTreeSet<Root> = BTreeSet::new();
    for (target, (em, en)) in exports {
        let mut frontier: BTreeSet<(Sym, Sym)> = BTreeSet::from([target]);
        for hop in 0..=MAX_HOPS {
            let mut next = BTreeSet::new();
            for t in &frontier {
                if let Some(ds) = decl.get(t) {
                    for &(p, flags) in ds {
                        roots.insert((em, en, p, t.1, flags));
                    }
                } else if hop < MAX_HOPS {
                    if let Some(bs) = bind.get(t) {
                        next.extend(bs.iter().copied());
                    }
                }
            }
            frontier = next;
        }
    }
    let mut out = BTreeSet::new();
    let mut by_owner: BTreeMap<Sym, Vec<Surface>> = BTreeMap::new();
    for r in &roots {
        match rules::surface(w, r) {
            SurfaceOrIssue::Surface(s) => by_owner.entry(s.0).or_default().push(s),
            SurfaceOrIssue::Mixed(f) => {
                out.insert(f);
            }
            SurfaceOrIssue::None => {}
        }
    }
    let mut fams: BTreeMap<(Sym, Vec<(u8, Sym, Sym)>), Vec<(Sym, (Sym, Sym))>> = BTreeMap::new();
    for (owner, mut ss) in by_owner {
        ss.sort_unstable();
        ss.dedup();
        for f in rules::owner_families(w, owner, &ss) {
            let (base, imp, ex): Family = f;
            fams.entry((base, imp)).or_default().push((owner, ex));
        }
    }
    for ((base, imp), owners) in fams {
        out.extend(rules::family_issues(w, base, &imp, &owners));
    }
    out
}

fn r3(w: &mut World, files: &[(Sym, Cid)]) -> BTreeSet<R3F> {
    let mut targets: BTreeMap<(Sym, Sym), BTreeSet<Node>> = BTreeMap::new();
    let mut edges: BTreeSet<(Node, Node)> = BTreeSet::new();
    let mut ext: Vec<((Sym, Sym), Node)> = Vec::new();
    let mut seeds: BTreeSet<(Node, Sym)> = BTreeSet::new();
    for &(p, c) in files {
        for (k, n) in w.r3_target(p, c) {
            targets.entry(k).or_default().insert(n);
        }
        let (local, e) = w.r3_calls(p, c);
        edges.extend(local);
        ext.extend(e);
        seeds.extend(w.r3_seeds(p, c));
    }
    for (k, caller) in ext {
        if let Some(ts) = targets.get(&k) {
            for &t in ts {
                edges.insert((caller, t));
            }
        }
    }
    let mut callers: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
    for &(a, b) in &edges {
        callers.entry(b).or_default().push(a);
    }
    // Backward closure from the seeds, per family.
    let mut reach: BTreeSet<(Node, Sym)> = seeds.clone();
    let mut queue: VecDeque<(Node, Sym)> = seeds.into_iter().collect();
    while let Some((n, f)) = queue.pop_front() {
        for &c in callers.get(&n).map(|v| v.as_slice()).unwrap_or(&[]) {
            if reach.insert((c, f)) {
                queue.push_back((c, f));
            }
        }
    }
    let mut out = BTreeSet::new();
    for &(n, f) in &reach {
        for &c in callers.get(&n).map(|v| v.as_slice()).unwrap_or(&[]) {
            if w.info(c.0).r3_scope {
                out.insert((c.0, c.1, f));
            }
        }
    }
    out
}

/// R2 hop census: for every export root, the number of re-export hops its resolution follows
/// (unbounded, up to 64, cycle-safe). Returns (hops -> resolved targets, unresolved after 64).
pub fn r2_hop_census(w: &mut World, files: &[(Sym, Cid)]) -> (BTreeMap<usize, usize>, usize) {
    let mut decl: BTreeSet<(Sym, Sym)> = BTreeSet::new();
    let mut bind: BTreeMap<(Sym, Sym), BTreeSet<(Sym, Sym)>> = BTreeMap::new();
    let mut exports: Vec<(Sym, Sym)> = Vec::new();
    for &(p, c) in files {
        for (k, _) in w.r2_decl(p, c) {
            decl.insert(k);
        }
        for (k, v) in w.r2_bind(p, c) {
            bind.entry(k).or_default().insert(v);
        }
        exports.extend(w.r2_export(p, c).into_iter().map(|(t, _)| t));
    }
    let mut hist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut open = 0;
    for t in exports {
        let mut frontier: BTreeSet<(Sym, Sym)> = BTreeSet::from([t]);
        let mut seen = frontier.clone();
        let mut hop = 0;
        let mut deepest = None;
        while !frontier.is_empty() && hop <= 64 {
            let mut next = BTreeSet::new();
            for t in &frontier {
                if decl.contains(t) {
                    deepest = Some(hop);
                } else if let Some(bs) = bind.get(t) {
                    next.extend(bs.iter().copied().filter(|x| seen.insert(*x)));
                }
            }
            frontier = next;
            hop += 1;
        }
        match deepest {
            Some(h) => *hist.entry(h).or_default() += 1,
            None if !frontier.is_empty() => open += 1,
            None => *hist.entry(usize::MAX).or_default() += 1,
        }
    }
    (hist, open)
}
