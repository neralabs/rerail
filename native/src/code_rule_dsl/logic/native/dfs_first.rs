//! `dfs_first` and `dfs_first_memo` (`core.rr`, data-layer/DESIGN.md addition 7): the first target a depth-first
//! search from each root reaches, set-based until a cycle.
//!
//! From the spike `docs/rule-syntax/spikes/dfs-first/native` (checked there against a port of the Data-layer
//! branch's `ordered_first.rs` on 40,000 random graphs and LEG-083's real call graph). A node's edges are tried in
//! increasing `ord` (then `dst`); each node is entered once and a node on the current path is skipped; a target is
//! a hit when an edge reaches it, so a root is never its own hit. Four set passes answer every root whose chain of
//! first edges ends at a target:
//!
//! 1. `live`: the nodes that reach a target (a backward closure).
//! 2. `next`: a node's first edge into a live node.
//! 3. `first_of`: where the chain of `next` edges ends.
//! 4. the knot: live non-target nodes on a cycle (Tarjan).
//!
//! Only a root whose chain enters the knot is searched, stepping into a node only while its chain meets the knot.
//!
//! `dfs_first` searches each root afresh. `dfs_first_memo` is the scripts' variant: roots in their given order,
//! one memo shared by all, so a node finished while a cycle was open is remembered as reaching nothing.
use std::collections::{HashMap, HashSet};

const NONE: u32 = u32::MAX;

/// A graph over dense ids: out-edges of `u` are `off[u]..off[u + 1]`, in (ord, dst) order.
struct Graph {
    n: usize,
    off: Vec<usize>,
    dst: Vec<u32>,
    ord: Vec<i64>,
    target: Vec<bool>,
}

impl Graph {
    fn new(n: usize, mut edges: Vec<(u32, u32, i64)>, targets: &[u32]) -> Self {
        edges.sort_unstable_by_key(|e| (e.0, e.2, e.1));
        edges.dedup();
        let mut off = vec![0; n + 1];
        for e in &edges {
            off[e.0 as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut target = vec![false; n];
        for &t in targets {
            target[t as usize] = true;
        }
        Graph { n, off, dst: edges.iter().map(|e| e.1).collect(), ord: edges.iter().map(|e| e.2).collect(), target }
    }
    fn out(&self, u: u32) -> std::ops::Range<usize> {
        self.off[u as usize]..self.off[u as usize + 1]
    }
}

/// A root's answer: the target hit and the ord of the root's edge that starts the path.
type Found = Option<(u32, i64)>;

struct Prep {
    live: Vec<bool>,
    /// The edge index of u's first edge into a live node, or NONE.
    nxt: Vec<u32>,
    /// The target u's chain of `nxt` edges ends at, or NONE.
    first: Vec<u32>,
    /// The first knot node on u's chain, or NONE: u is answered by the set passes.
    ent: Vec<u32>,
}

fn prepare(g: &Graph) -> Prep {
    let n = g.n;
    // 1. live: backward closure from the targets, over a reverse CSR.
    let mut roff = vec![0usize; n + 1];
    for &d in &g.dst {
        roff[d as usize + 1] += 1;
    }
    for i in 0..n {
        roff[i + 1] += roff[i];
    }
    let mut rsrc = vec![0u32; g.dst.len()];
    let mut fill = roff.clone();
    for u in 0..n {
        for e in g.out(u as u32) {
            let d = g.dst[e] as usize;
            rsrc[fill[d]] = u as u32;
            fill[d] += 1;
        }
    }
    let mut live = vec![false; n];
    let mut queue: Vec<u32> = (0..n as u32).filter(|&v| g.target[v as usize]).collect();
    for &v in &queue {
        live[v as usize] = true;
    }
    while let Some(v) = queue.pop() {
        for &u in &rsrc[roff[v as usize]..roff[v as usize + 1]] {
            if !live[u as usize] {
                live[u as usize] = true;
                queue.push(u);
            }
        }
    }
    drop(rsrc);
    // 2. next: the first edge into a live node; an edge to itself is skipped unless it is a hit.
    let mut nxt = vec![NONE; n];
    for u in 0..n {
        for e in g.out(u as u32) {
            let d = g.dst[e] as usize;
            if live[d] && (d != u || g.target[d]) {
                nxt[u] = e as u32;
                break;
            }
        }
    }
    // 3. first_of: resolve the functional graph of next. A chain that loops, and every chain that runs into the
    //    loop, gets NONE; the knot sends those roots to the search.
    let mut first = vec![NONE; n];
    let mut state = vec![0u8; n]; // 0 new, 1 on the current walk, 2 resolved
    let mut walk = Vec::new();
    for s in 0..n {
        let mut v = s;
        let ans = loop {
            match state[v] {
                2 => break first[v],
                1 => break NONE,
                _ => {}
            }
            state[v] = 1;
            walk.push(v);
            if nxt[v] == NONE {
                break NONE;
            }
            let d = g.dst[nxt[v] as usize] as usize;
            if g.target[d] {
                break d as u32;
            }
            v = d;
        };
        for w in walk.drain(..) {
            first[w] = ans;
            state[w] = 2;
        }
    }
    let knot = knot(g, &live);
    let inner = |u: usize| live[u] && !g.target[u];
    // 5. ent: the first knot node on each chain. Every next-loop lies in the knot, so walks end.
    let mut ent = vec![NONE; n];
    let mut done = vec![false; n];
    for s in 0..n {
        let mut v = s;
        walk.clear();
        let ans = loop {
            if done[v] {
                break ent[v];
            }
            if !inner(v) {
                done[v] = true;
                break NONE;
            }
            if knot[v] {
                ent[v] = v as u32;
                done[v] = true;
                break v as u32;
            }
            walk.push(v);
            let d = if nxt[v] == NONE { usize::MAX } else { g.dst[nxt[v] as usize] as usize };
            if d == usize::MAX || g.target[d] {
                break NONE;
            }
            v = d;
        };
        for &w in &walk {
            ent[w] = ans;
            done[w] = true;
        }
    }
    Prep { live, nxt, first, ent }
}

/// 4. Tarjan over live non-target nodes and the edges between them, read from the CSR directly.
fn knot(g: &Graph, live: &[bool]) -> Vec<bool> {
    let n = g.n;
    let inner = |u: usize| live[u] && !g.target[u];
    let mut knot = vec![false; n];
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on = vec![false; n];
    let mut stk: Vec<u32> = Vec::new();
    let mut work: Vec<(u32, usize)> = Vec::new();
    let mut counter = 0u32;
    let mut comp = Vec::new();
    for v0 in 0..n {
        if !inner(v0) || index[v0] != u32::MAX {
            continue;
        }
        index[v0] = counter;
        low[v0] = counter;
        counter += 1;
        stk.push(v0 as u32);
        on[v0] = true;
        work.push((v0 as u32, g.off[v0]));
        while !work.is_empty() {
            let top = work.len() - 1;
            let v = work[top].0 as usize;
            let mut child = None;
            while work[top].1 < g.off[v + 1] {
                let w = g.dst[work[top].1] as usize;
                work[top].1 += 1;
                if w == v || !inner(w) {
                    continue;
                }
                if index[w] == u32::MAX {
                    child = Some(w);
                    break;
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            }
            if let Some(w) = child {
                index[w] = counter;
                low[w] = counter;
                counter += 1;
                stk.push(w as u32);
                on[w] = true;
                work.push((w as u32, g.off[w]));
                continue;
            }
            work.pop();
            if let Some(&(p, _)) = work.last() {
                low[p as usize] = low[p as usize].min(low[v]);
            }
            if low[v] == index[v] {
                comp.clear();
                loop {
                    let w = stk.pop().expect("Tarjan's stack") as usize;
                    on[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                if comp.len() > 1 {
                    for &w in &comp {
                        knot[w] = true;
                    }
                }
            }
        }
    }
    knot
}

/// The depth-first search from `root`, stepping into a node only while its chain meets the knot; at a node the
/// set passes answered it takes that answer.
fn search(g: &Graph, p: &Prep, root: u32, memo: &mut HashMap<u32, Found>) -> Found {
    if let Some(f) = memo.get(&root) {
        return *f;
    }
    let mut stack: Vec<(u32, usize, Option<i64>)> = vec![(root, g.off[root as usize], None)];
    let mut visiting = HashSet::from([root]);
    loop {
        let top = stack.len() - 1;
        let (node, next, _) = stack[top];
        if next >= g.off[node as usize + 1] {
            stack.pop();
            visiting.remove(&node);
            memo.insert(node, None);
            if stack.is_empty() {
                return None;
            }
            continue;
        }
        stack[top].1 += 1;
        let (child, ord) = (g.dst[next], g.ord[next]);
        let c = child as usize;
        let found = if g.target[c] {
            Some((child, ord))
        } else if !p.live[c] || visiting.contains(&child) {
            continue;
        } else if let Some(m) = memo.get(&child) {
            m.map(|(s, _)| (s, ord))
        } else if p.ent[c] == NONE {
            Some((p.first[c], ord))
        } else {
            visiting.insert(child);
            stack.push((child, g.off[c], Some(ord)));
            continue;
        };
        if let Some((s, mut ord)) = found {
            while let Some((f, _, via)) = stack.pop() {
                visiting.remove(&f);
                memo.insert(f, Some((s, ord)));
                if let Some(v) = via {
                    ord = v;
                }
            }
            return Some((s, ord));
        }
    }
}

fn answer(g: &Graph, p: &Prep, roots: &[u32], share: bool) -> Vec<Found> {
    let mut memo = HashMap::new();
    let mut cache: HashMap<u32, Found> = HashMap::new();
    let mut out = Vec::with_capacity(roots.len());
    for &r in roots {
        let ru = r as usize;
        let ans = if g.target[ru] {
            // A target root: only an edge reaches a hit, so it is searched.
            if share { search(g, p, r, &mut memo) } else { search(g, p, r, &mut HashMap::new()) }
        } else if !p.live[ru] {
            None
        } else if p.ent[ru] == NONE {
            Some((p.first[ru], g.ord[p.nxt[ru] as usize]))
        } else {
            let u = p.ent[ru];
            let a = if share {
                let a = search(g, p, u, &mut memo);
                if a.is_none() {
                    out.push(search(g, p, r, &mut memo));
                    continue;
                }
                a
            } else {
                match cache.get(&u) {
                    Some(a) => *a,
                    None => {
                        let a = search(g, p, u, &mut HashMap::new());
                        cache.insert(u, a);
                        a
                    }
                }
            };
            if u == r { a } else { a.map(|(s, _)| (s, g.ord[p.nxt[ru] as usize])) }
        };
        out.push(ans);
    }
    out
}

/// `dfs_first(r, hit, first, edge, target)` over the rows of `edge` (src, dst, ord) and `target`: a row per root
/// with an answer. With `roots` (`dfs_first_memo`), its roots in that order sharing one memo; else every node with
/// an edge, each searched afresh. Values are the relations' own (node ids, numbers or atoms).
pub(in crate::code_rule_dsl) fn dfs_first(edges: &[(i64, i64, i64)], targets: &[i64], roots: Option<&[i64]>) -> Vec<(i64, i64, i64)> {
    let mut ids: HashMap<i64, u32> = HashMap::new();
    let mut values: Vec<i64> = Vec::new();
    let mut id = |v: i64, values: &mut Vec<i64>| {
        *ids.entry(v).or_insert_with(|| {
            values.push(v);
            (values.len() - 1) as u32
        })
    };
    let dense: Vec<(u32, u32, i64)> = edges.iter().map(|&(s, d, o)| (id(s, &mut values), id(d, &mut values), o)).collect();
    let targets: Vec<u32> = targets.iter().map(|&t| id(t, &mut values)).collect();
    let (roots, share): (Vec<u32>, bool) = match roots {
        Some(roots) => (roots.iter().map(|&r| id(r, &mut values)).collect(), true),
        None => {
            let mut sources: Vec<u32> = dense.iter().map(|e| e.0).collect();
            sources.sort_unstable();
            sources.dedup();
            (sources, false)
        }
    };
    let g = Graph::new(values.len(), dense, &targets);
    let p = prepare(&g);
    let mut out: Vec<(i64, i64, i64)> = roots
        .iter()
        .zip(answer(&g, &p, &roots, share))
        .filter_map(|(&r, found)| found.map(|(hit, ord)| (values[r as usize], values[hit as usize], ord)))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The Data-layer branch's `ordered_first.rs` `Graph::first`, as the spike ports it line for line (edge mode):
    /// the reference the set-based plan must equal.
    fn oracle_first(g: &Graph, root: u32, memo: &mut HashMap<u32, Found>) -> Found {
        if let Some(f) = memo.get(&root) {
            return *f;
        }
        let mut stack: Vec<(u32, usize, Option<i64>)> = vec![(root, g.off[root as usize], None)];
        let mut visiting = HashSet::from([root]);
        loop {
            let top = stack.len() - 1;
            let (node, next, _) = stack[top];
            if next >= g.off[node as usize + 1] {
                stack.pop();
                visiting.remove(&node);
                memo.insert(node, None);
                if stack.is_empty() {
                    return None;
                }
                continue;
            }
            stack[top].1 += 1;
            let (child, ord) = (g.dst[next], g.ord[next]);
            let found = if g.target[child as usize] {
                Some((child, ord))
            } else if visiting.contains(&child) {
                continue;
            } else if let Some(c) = memo.get(&child) {
                c.map(|(s, _)| (s, ord))
            } else {
                visiting.insert(child);
                stack.push((child, g.off[child as usize], Some(ord)));
                continue;
            };
            if let Some((s, mut ord)) = found {
                while let Some((f, _, via)) = stack.pop() {
                    visiting.remove(&f);
                    memo.insert(f, Some((s, ord)));
                    if let Some(v) = via {
                        ord = v;
                    }
                }
                return Some((s, ord));
            }
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Random graphs (self-loops, duplicate edges, tied orders): the set-based plan answers every root as the
    /// oracle's search does, with a fresh memo per root and with one shared in root order.
    #[test]
    fn set_based_plan_equals_the_scripts_search() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..4000 {
            let n = 1 + rng.below(if case % 4 == 0 { 40 } else { 9 }) as usize;
            let m = rng.below(3 * n as u64 + 1) as usize;
            let edges: Vec<(u32, u32, i64)> = (0..m).map(|_| (rng.below(n as u64) as u32, rng.below(n as u64) as u32, rng.below(4) as i64)).collect();
            let targets: Vec<u32> = (0..n as u32).filter(|_| rng.below(4) == 0).collect();
            let g = Graph::new(n, edges.clone(), &targets);
            let p = prepare(&g);
            let roots: Vec<u32> = (0..n as u32).collect();
            let fresh: Vec<Found> = roots.iter().map(|&r| oracle_first(&g, r, &mut HashMap::new())).collect();
            assert_eq!(answer(&g, &p, &roots, false), fresh, "case {case}, fresh memo: {edges:?} targets {targets:?}");
            let mut memo = HashMap::new();
            let shared: Vec<Found> = roots.iter().map(|&r| oracle_first(&g, r, &mut memo)).collect();
            assert_eq!(answer(&g, &p, &roots, true), shared, "case {case}, shared memo: {edges:?} targets {targets:?}");
        }
    }

    /// The relation's rows over arbitrary ids: a path to the target through the first edge by ord, a root that
    /// reaches nothing has no row, and a cycle is searched.
    #[test]
    fn rows_over_relation_values() {
        // 10 -> 40 (ord 0) -> 10 (a cycle back), 10 -> 20 (ord 1) -> 30 target; 50 reaches nothing. From 10 the
        // ord-0 edge is tried first and leads only back, so the path starts with the ord-1 edge.
        let edges = [(10, 20, 1), (20, 30, 5), (10, 40, 0), (40, 10, 2), (50, 60, 0)];
        assert_eq!(dfs_first(&edges, &[30], None), [(10, 30, 1), (20, 30, 5), (40, 30, 2)]);
        // Shared memo in root order: 40 first (its search through 10 is remembered), then 10, then 50.
        assert_eq!(dfs_first(&edges, &[30], Some(&[40, 10, 50])), [(10, 30, 1), (40, 30, 2)]);
    }
}
