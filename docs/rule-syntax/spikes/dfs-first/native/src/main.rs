//! dfs_first, native: the set-based plan of data-layer/DESIGN.md addition 7, with no SQL and no
//! materialised tables, checked against a port of the Data-layer branch's ordered_first.rs.
//!
//!   cargo run --release -- check 20000          differential test, small and large graphs
//!   cargo run --release -- state 20000          supplied memo/visiting = an edge rewrite
//!   cargo run --release -- par 5000             threaded passes and GPU steps 1-3 = sequential
//!   cargo run --release -- bench 1000000 0.05 [giant]
//!   cargo run --release -- real leg.graph       LEG-083 on the data-layer repo (../realgraph.py)
//!
//! Std only, plus Metal through build.rs. CPU time is getrusage(RUSAGE_SELF), user+sys.

use std::collections::{HashMap, HashSet};

mod gpu;
mod par;
mod real;

pub(crate) const NONE: u32 = u32::MAX;

// ------------------------------------------------------------------ graph --
#[derive(Clone)]
pub(crate) struct Graph {
    pub(crate) n: usize,
    pub(crate) off: Vec<usize>,            // CSR: out-edges of u are off[u]..off[u+1], in (ord, dst) order
    pub(crate) dst: Vec<u32>,
    pub(crate) loc: Vec<i64>,
    pub(crate) sink: Vec<Option<i64>>,     // the target's own location
}

impl Graph {
    pub(crate) fn new(n: usize, mut edges: Vec<(u32, u32, i64, i64)>, sinks: &[(u32, i64)]) -> Graph {
        edges.sort_by_key(|e| (e.0, e.2, e.1));
        let mut off = vec![0; n + 1];
        for e in &edges {
            off[e.0 as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut sink = vec![None; n];
        for &(v, l) in sinks {
            sink[v as usize] = Some(l);
        }
        Graph { n, off, dst: edges.iter().map(|e| e.1).collect(), loc: edges.iter().map(|e| e.3).collect(), sink }
    }
    pub(crate) fn out(&self, u: u32) -> std::ops::Range<usize> {
        self.off[u as usize]..self.off[u as usize + 1]
    }
}

pub(crate) type Found = Option<(u32, i64)>;

// ----------------------------------------------------------------- oracle --
// ordered_first.rs `Graph::first`, line for line, with its supplied state: `blocked` (the initial
// visiting set) and an initial memo.
fn oracle_first(g: &Graph, root: u32, on_edge: bool, blocked: &HashSet<u32>, memo: &mut HashMap<u32, Found>, steps: &mut u64) -> Found {
    *steps += 1;
    if let Some(f) = memo.get(&root) {
        return *f;
    }
    if blocked.contains(&root) {
        memo.insert(root, None);
        return None;
    }
    if !on_edge && let Some(l) = g.sink[root as usize] {
        memo.insert(root, Some((root, l)));
        return Some((root, l));
    }
    let mut stack: Vec<(u32, usize, Option<i64>)> = vec![(root, g.off[root as usize], None)];
    let mut visiting = HashSet::from([root]);
    loop {
        *steps += 1;
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
        let (child, loc) = (g.dst[next], g.loc[next]);
        let found = if on_edge && g.sink[child as usize].is_some() {
            Some((child, loc))
        } else if visiting.contains(&child) || blocked.contains(&child) {
            continue;
        } else if let Some(c) = memo.get(&child) {
            c.map(|(s, _)| (s, loc))
        } else if !on_edge && let Some(d) = g.sink[child as usize] {
            memo.insert(child, Some((child, d)));
            Some((child, loc))
        } else {
            visiting.insert(child);
            stack.push((child, g.off[child as usize], Some(loc)));
            continue;
        };
        if let Some((s, mut loc)) = found {
            while let Some((f, _, via)) = stack.pop() {
                visiting.remove(&f);
                memo.insert(f, Some((s, loc)));
                if let Some(v) = via {
                    loc = v;
                }
            }
            return Some((s, loc));
        }
    }
}

pub(crate) fn oracle(g: &Graph, roots: &[u32], node_mode: bool, share: bool, blocked: &HashSet<u32>, initial: &HashMap<u32, Found>) -> (Vec<Found>, u64) {
    let mut steps = 0;
    let mut memo = initial.clone();
    let out = roots
        .iter()
        .map(|&r| {
            if share {
                oracle_first(g, r, !node_mode, blocked, &mut memo, &mut steps)
            } else {
                oracle_first(g, r, !node_mode, blocked, &mut initial.clone(), &mut steps)
            }
        })
        .collect();
    (out, steps)
}

// -------------------------------------------------------------- set-based --
pub(crate) struct Prep {
    pub(crate) live: Vec<bool>,
    pub(crate) nxt: Vec<u32>,     // the edge index of u's first edge into a live node, or NONE
    pub(crate) first: Vec<u32>,   // first_of: the target u's next chain ends at, or NONE
    pub(crate) knot: Vec<bool>,   // on a cycle of live non-sink nodes
    pub(crate) ent: Vec<u32>,     // the first knot node on u's chain, or NONE: u is "pure"
}

pub(crate) fn cpu() -> f64 {
    #[repr(C)]
    struct Tv {
        sec: i64,
        usec: i32,
    }
    #[repr(C)]
    struct Ru {
        utime: Tv,
        stime: Tv,
        rest: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Ru) -> i32;
    }
    let mut r = std::mem::MaybeUninit::<Ru>::zeroed();
    unsafe {
        getrusage(0, r.as_mut_ptr());
        let r = r.assume_init();
        r.utime.sec as f64 + r.utime.usec as f64 / 1e6 + r.stime.sec as f64 + r.stime.usec as f64 / 1e6
    }
}

pub(crate) fn prepare(g: &Graph, times: &mut Vec<(&'static str, f64)>) -> Prep {
    let n = g.n;
    let mut t = cpu();
    let mut lap = |name: &'static str, times: &mut Vec<(&'static str, f64)>| {
        let now = cpu();
        times.push((name, now - t));
        t = now;
    };
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
    let mut queue: Vec<u32> = (0..n as u32).filter(|&v| g.sink[v as usize].is_some()).collect();
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
    lap("live", times);
    // 2. next: the first edge into a live node; a call to itself is skipped unless it is a hit.
    let mut nxt = vec![NONE; n];
    for u in 0..n {
        for e in g.out(u as u32) {
            let d = g.dst[e] as usize;
            if live[d] && (d != u || g.sink[d].is_some()) {
                nxt[u] = e as u32;
                break;
            }
        }
    }
    lap("next", times);
    // 3. first_of: resolve the functional graph of next. A chain that loops, and every chain that
    //    runs into the loop, gets NONE; the knot pass sends those roots to the search.
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
            if g.sink[d].is_some() {
                break d as u32;
            }
            v = d;
        };
        for w in walk.drain(..) {
            first[w] = ans;
            state[w] = 2;
        }
    }
    lap("first_of", times);
    let knot = knot_pass(g, &live);
    lap("knot", times);
    let inner = |u: usize| live[u] && g.sink[u].is_none();
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
            if d == usize::MAX || g.sink[d].is_some() {
                break NONE;
            }
            v = d;
        };
        for &w in &walk {
            ent[w] = ans;
            done[w] = true;
        }
    }
    lap("ent", times);
    Prep { live, nxt, first, knot, ent }
}

/// Step 4: Tarjan over live non-sink nodes and the edges between them, read from the CSR directly.
/// Sequential: SCCs are the one step the GPU and the threads leave to a single core.
pub(crate) fn knot_pass(g: &Graph, live: &[bool]) -> Vec<bool> {
    let n = g.n;
    let inner = |u: usize| live[u] && g.sink[u].is_none();
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
                    let w = stk.pop().unwrap() as usize;
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

pub(crate) struct Run<'a> {
    pub(crate) g: &'a Graph,
    pub(crate) p: &'a Prep,
    pub(crate) steps: u64,
    pub(crate) searches: u64,
}

/// ordered_first.rs's search, stepping into a node only if its chain meets the knot; at a pure node
/// it takes the set-based answer.
pub(crate) fn search(g: &Graph, p: &Prep, root: u32, on_edge: bool, memo: &mut HashMap<u32, Found>, steps: &mut u64) -> Found {
    *steps += 1;
    if let Some(f) = memo.get(&root) {
        return *f;
    }
    let mut stack: Vec<(u32, usize, Option<i64>)> = vec![(root, g.off[root as usize], None)];
    let mut visiting = HashSet::from([root]);
    loop {
        *steps += 1;
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
        let (child, loc) = (g.dst[next], g.loc[next]);
        let c = child as usize;
        let found = if on_edge && g.sink[c].is_some() {
            Some((child, loc))
        } else if !p.live[c] || visiting.contains(&child) {
            continue;
        } else if let Some(m) = memo.get(&child) {
            m.map(|(s, _)| (s, loc))
        } else if g.sink[c].is_some() {
            Some((child, loc))
        } else if p.ent[c] == NONE {
            Some((p.first[c], loc))
        } else {
            visiting.insert(child);
            stack.push((child, g.off[c], Some(loc)));
            continue;
        };
        if let Some((s, mut loc)) = found {
            while let Some((f, _, via)) = stack.pop() {
                visiting.remove(&f);
                memo.insert(f, Some((s, loc)));
                if let Some(v) = via {
                    loc = v;
                }
            }
            return Some((s, loc));
        }
    }
}

impl Run<'_> {
    fn search(&mut self, root: u32, on_edge: bool, memo: &mut HashMap<u32, Found>) -> Found {
        self.searches += 1;
        search(self.g, self.p, root, on_edge, memo, &mut self.steps)
    }

    pub(crate) fn run(&mut self, roots: &[u32], node_mode: bool, share: bool) -> Vec<Found> {
        let (g, p) = (self.g, self.p);
        let on_edge = !node_mode;
        let mut memo = HashMap::new();
        let mut cache: HashMap<u32, Found> = HashMap::new();
        let mut out = Vec::with_capacity(roots.len());
        for &r in roots {
            let ru = r as usize;
            let ans = if let Some(l) = g.sink[ru] {
                if on_edge {
                    if share { self.search(r, true, &mut memo) } else { self.search(r, true, &mut HashMap::new()) }
                } else {
                    Some((r, l))
                }
            } else if !p.live[ru] {
                None
            } else if p.ent[ru] == NONE {
                Some((p.first[ru], g.loc[p.nxt[ru] as usize]))
            } else {
                let u = p.ent[ru];
                let a = if share {
                    let a = self.search(u, on_edge, &mut memo);
                    if a.is_none() {
                        out.push(self.search(r, on_edge, &mut memo));
                        continue;
                    }
                    a
                } else {
                    match cache.get(&u) {
                        Some(a) => *a,
                        None => {
                            let a = self.search(u, on_edge, &mut HashMap::new());
                            cache.insert(u, a);
                            a
                        }
                    }
                };
                if u == r { a } else { a.map(|(s, _)| (s, g.loc[p.nxt[ru] as usize])) }
            };
            out.push(ans);
        }
        out
    }
}

// ---------------------------------------------------- supplied state rewrite --
// The producer's supplied visiting set and memo, as a rewrite of the edge and target relations,
// so the language needs no extra arguments (DESIGN.md, addition 7). Seeds name targets.
//   blocked b: drop b's edges; drop edges into b unless b is a target in edge mode;
//              in node mode b is no longer a target.
//   seed v -> (s, l): v's edges become one edge v -> s at location l; in node mode v is no
//              longer a target (the memo is read before v's own target test).
pub(crate) fn rewrite(n: usize, edges: &[(u32, u32, i64, i64)], sinks: &[(u32, i64)], node_mode: bool,
           blocked: &HashSet<u32>, seed: &HashMap<u32, (u32, i64)>) -> (Vec<(u32, u32, i64, i64)>, Vec<(u32, i64)>) {
    let is_sink: HashSet<u32> = sinks.iter().map(|s| s.0).collect();
    let mut e2: Vec<_> = edges
        .iter()
        .filter(|e| !blocked.contains(&e.0) && !seed.contains_key(&e.0))
        .filter(|e| !blocked.contains(&e.1) || (!node_mode && is_sink.contains(&e.1)))
        .copied()
        .collect();
    for (&v, &(s, l)) in seed {
        e2.push((v, s, i64::MIN, l));
    }
    let s2 = sinks
        .iter()
        .filter(|s| !node_mode || (!blocked.contains(&s.0) && !seed.contains_key(&s.0)))
        .copied()
        .collect();
    let _ = n;
    (e2, s2)
}

// --------------------------------------------------------------- harness --
pub(crate) struct Rng(u64);
impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub(crate) fn below(&mut self, k: u64) -> u64 {
        self.next() % k.max(1)
    }
    pub(crate) fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

pub(crate) fn random_graph(rng: &mut Rng, large: bool) -> (usize, Vec<(u32, u32, i64, i64)>, Vec<(u32, i64)>, Vec<u32>) {
    let n = if large { 10 + rng.below(31) } else { 1 + rng.below(9) } as usize;
    let m = if large { n as u64 + rng.below(2 * n as u64 + 1) } else { rng.below(3 * n as u64 + 1) };
    let edges = (0..m).map(|_| (rng.below(n as u64) as u32, rng.below(n as u64) as u32, rng.below(5) as i64, 1 + rng.below(99) as i64)).collect();
    let p = if large { 0.1 } else { 0.25 };
    let mut sinks = Vec::new();
    for v in 0..n as u32 {
        if rng.chance(p) {
            sinks.push((v, 100 + rng.below(100) as i64));
        }
    }
    let mut roots: Vec<u32> = (0..n as u32).collect();
    for i in (1..n).rev() {
        roots.swap(i, rng.below(i as u64 + 1) as usize);
    }
    roots.truncate(1 + rng.below(n as u64) as usize);
    (n, edges, sinks, roots)
}

fn check(cases: usize) -> bool {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let (none, empty) = (HashSet::new(), HashMap::new());
    for large in [false, true] {
        for i in 0..cases {
            let (n, edges, sinks, roots) = random_graph(&mut rng, large);
            let g = Graph::new(n, edges.clone(), &sinks);
            let p = prepare(&g, &mut Vec::new());
            for node_mode in [true, false] {
                for share in [false, true] {
                    let (want, _) = oracle(&g, &roots, node_mode, share, &none, &empty);
                    let got = Run { g: &g, p: &p, steps: 0, searches: 0 }.run(&roots, node_mode, share);
                    if want != got {
                        println!("DISAGREE large={large} case {i} node_mode={node_mode} share={share}\n edges {edges:?}\n sinks {sinks:?}\n roots {roots:?}\n want {want:?}\n got  {got:?}");
                        return false;
                    }
                }
            }
        }
    }
    println!("ok: {cases} small and {cases} large graphs x 2 modes x 2 memo variants");
    true
}

fn check_state(cases: usize) -> bool {
    let mut rng = Rng(0xD1B54A32D192ED03);
    for i in 0..cases {
        let (n, edges, sinks, roots) = random_graph(&mut rng, i % 2 == 1);
        if sinks.is_empty() {
            continue;
        }
        let mut blocked = HashSet::new();
        let mut seed: HashMap<u32, (u32, i64)> = HashMap::new();
        // A seed names a target that is itself neither blocked nor seeded, as a memo entry
        // the search wrote would.
        let anchor = sinks[rng.below(sinks.len() as u64) as usize].0;
        for v in 0..n as u32 {
            if v == anchor {
                continue;
            }
            if rng.chance(0.15) {
                blocked.insert(v);
            } else if rng.chance(0.15) {
                seed.insert(v, (anchor, 1000 + rng.below(99) as i64));
            }
        }
        let initial: HashMap<u32, Found> = seed.iter().map(|(&v, &f)| (v, Some(f))).collect();
        let g = Graph::new(n, edges.clone(), &sinks);
        for node_mode in [true, false] {
            for share in [false, true] {
                let (want, _) = oracle(&g, &roots, node_mode, share, &blocked, &initial);
                let (e2, s2) = rewrite(n, &edges, &sinks, node_mode, &blocked, &seed);
                let g2 = Graph::new(n, e2, &s2);
                let p2 = prepare(&g2, &mut Vec::new());
                // Blocked roots answer None; the rest are ordinary roots of the rewritten graph.
                let kept: Vec<u32> = roots.iter().copied().filter(|r| !blocked.contains(r)).collect();
                let got_kept = Run { g: &g2, p: &p2, steps: 0, searches: 0 }.run(&kept, node_mode, share);
                let mut it = got_kept.into_iter();
                let got: Vec<Found> = roots.iter().map(|r| if blocked.contains(r) { None } else { it.next().unwrap() }).collect();
                if want != got {
                    println!("DISAGREE state case {i} node_mode={node_mode} share={share}\n edges {edges:?}\n sinks {sinks:?}\n roots {roots:?}\n blocked {blocked:?}\n seed {seed:?}\n want {want:?}\n got  {got:?}");
                    return false;
                }
            }
        }
    }
    println!("ok: {cases} graphs with supplied visiting sets and memos, x 2 modes x 2 memo variants");
    true
}

/// The parallel passes and the GPU's steps 1-3 give the sequential arrays and answers.
fn check_par(cases: usize, threads: usize) -> bool {
    let mut rng = Rng(0xA0761D6478BD642F);
    let gpu = gpu::init();
    for large in [false, true] {
        for i in 0..cases {
            let (n, edges, sinks, roots) = random_graph(&mut rng, large);
            let g = Graph::new(n, edges.clone(), &sinks);
            let p = prepare(&g, &mut Vec::new());
            let q = par::prepare_par(&g, threads, &mut Vec::new());
            let gpu_bad = if gpu.is_some() && i % 10 == 0 { gpu::differs(&gpu::prepare(&g), &p) } else { None };
            let mut bad = if !par::same(&p, &q) { Some("threaded arrays".to_string()) } else { gpu_bad.map(|x| format!("GPU {x}")) };
            for node_mode in [true, false] {
                for share in [false, true] {
                    let want = Run { g: &g, p: &p, steps: 0, searches: 0 }.run(&roots, node_mode, share);
                    if par::run_par(&g, &q, &roots, node_mode, share, threads).0 != want {
                        bad.get_or_insert(format!("threaded roots, node_mode={node_mode} share={share}"));
                    }
                }
            }
            if let Some(b) = bad {
                println!("DISAGREE ({b}) large={large} case {i}\n edges {edges:?}\n sinks {sinks:?}\n roots {roots:?}");
                return false;
            }
        }
    }
    println!("ok: {cases} small and {cases} large graphs, {threads} threads{}", if gpu.is_some() { "; GPU on every tenth" } else { "; no GPU" });
    true
}

fn bench(n: usize, back: f64, giant: bool, threads: usize) {
    let mut rng = Rng(1);
    let mut edges = Vec::new();
    for s in 0..n {
        for o in 0..(1 + rng.below(9)) {
            let d = if rng.chance(back) && s > 0 {
                let lo = if giant { s.saturating_sub(50) } else { s - s % 5 };
                lo + rng.below((s + 1 - lo) as u64) as usize
            } else {
                let u = (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
                (s + 1 + (-(1.0 - u).ln() * 200.0) as usize).min(n - 1)
            };
            edges.push((s as u32, d as u32, o as i64, (s * 10) as i64 + o as i64));
        }
    }
    let sinks: Vec<(u32, i64)> = (0..n as u32).filter(|_| rng.chance(0.01)).map(|v| (v, v as i64 * 10)).collect();
    let roots: Vec<u32> = (0..n as u32).collect();
    println!("{n} functions, {} calls, {} targets, back-edge fraction {back}, {}", edges.len(), sinks.len(), if giant { "giant cycle" } else { "module cycles" });
    let t = cpu();
    let g = Graph::new(n, edges, &sinks);
    println!("  load (sort into CSR)         {:7.3}s", cpu() - t);
    let mut times = Vec::new();
    let p = prepare(&g, &mut times);
    for (name, s) in &times {
        println!("  {name:28} {s:7.3}s");
    }
    let knot = p.knot.iter().filter(|&&k| k).count();
    let meets = p.ent.iter().filter(|&&e| e != NONE).count();
    println!("  knot {knot} nodes; chains meeting it {meets}");
    let (none, empty) = (HashSet::new(), HashMap::new());
    for share in [false, true] {
        let t = cpu();
        let mut r = Run { g: &g, p: &p, steps: 0, searches: 0 };
        let got = r.run(&roots, true, share);
        let t_run = cpu() - t;
        let label = if share { "shared memo (scripts)" } else { "fresh memo (dfs_first)" };
        print!("  {label:24} roots {t_run:7.3}s, {} search steps", r.steps);
        if share || n <= 20000 {
            let t = cpu();
            let (want, steps) = oracle(&g, &roots, true, share, &none, &empty);
            let t_or = cpu() - t;
            assert!(want == got, "disagreement");
            println!(" | oracle {t_or:7.3}s, {steps} steps, agrees");
        } else {
            println!(" | oracle not run (one search per root)");
        }
    }
    let mut times = Vec::new();
    let q = par::prepare_par(&g, threads, &mut times);
    let total: f64 = times.iter().map(|t| t.1).sum();
    println!("  {threads} threads, set passes {total:7.3}s CPU ({}), arrays {}",
             times.iter().map(|(n, s)| format!("{n} {s:.3}")).collect::<Vec<_>>().join(", "),
             if par::same(&p, &q) { "equal" } else { "DIFFER" });
    for share in [false, true] {
        let t = cpu();
        let (got, steps) = par::run_par(&g, &q, &roots, true, share, threads);
        let dt = cpu() - t;
        let want = Run { g: &g, p: &p, steps: 0, searches: 0 }.run(&roots, true, share);
        println!("  {threads} threads, {} roots {dt:7.3}s CPU, {steps} steps, {}",
                 if share { "shared" } else { "fresh " }, if got == want { "agrees" } else { "DISAGREES" });
    }
    if let Some(dev) = gpu::init() {
        let t = cpu();
        let o = gpu::prepare(&g);
        let host = cpu() - t;
        println!("  GPU ({dev}) steps 1-3: {:.4}s GPU, host {host:.3}s CPU (CSR copies, reverse CSR), {} waves, {} command buffers, {}",
                 o.gpu_seconds, o.waves, o.command_buffers, gpu::differs(&o, &p).map_or("arrays equal".into(), |x| format!("{x} DIFFERS")));
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let num = |i: usize, d: usize| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let threads = std::env::var("THREADS").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |x| x.get()));
    let ok = match a.get(1).map(String::as_str) {
        Some("par") => check_par(num(2, 5000), threads),
        Some("real") => real::real(a.get(2).expect("graph file"), threads),
        Some("check") => check(num(2, 20000)),
        Some("state") => check_state(num(2, 20000)),
        Some("bench") => {
            bench(num(2, 100000), a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.05), a.get(4).map(String::as_str) == Some("giant"), threads);
            true
        }
        _ => {
            eprintln!("usage: check N | state N | par N | bench NODES BACK [giant] | real FILE   (THREADS=k)");
            false
        }
    };
    std::process::exit(if ok { 0 } else { 1 });
}
