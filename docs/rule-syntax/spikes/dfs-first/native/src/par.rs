//! The set passes and root answering on several threads (std::thread::scope, no crates).
//!
//! Steps 1-3 and 5 are the same waves the GPU runs: a frontier per level for `live`, one pass per
//! node for `next`, and pointer jumping (Wyllie) for `first_of` and `ent`. Step 4, Tarjan, stays on
//! one core. Every array must equal the sequential `prepare`'s; `check_par` tests that.

use crate::{Found, Graph, NONE, Prep, cpu, knot_pass, search};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

fn chunks(n: usize, threads: usize) -> usize {
    n.div_ceil(threads.max(1)).max(1024)
}

/// Run `f(start, slice)` over `out` split into one chunk per thread.
fn par_fill<T: Send>(out: &mut [T], threads: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let c = chunks(out.len(), threads);
    std::thread::scope(|sc| {
        for (i, part) in out.chunks_mut(c).enumerate() {
            let f = &f;
            sc.spawn(move || f(i * c, part));
        }
    });
}

/// Pointer jumping over a functional graph. `ptr[v] == v` marks a terminal whose answer is
/// `val[v]`. After ceil(log2 n) + 1 rounds every chain that ends has reached its terminal; a node
/// still pointing at a non-terminal is on, or runs into, a loop and gets `NONE`.
fn jump(mut ptr: Vec<u32>, val: &[u32], threads: usize) -> Vec<u32> {
    let n = ptr.len();
    let rounds = usize::BITS - n.max(1).leading_zeros() + 1;
    let mut next = vec![0u32; n];
    for _ in 0..rounds {
        {
            let p = &ptr;
            par_fill(&mut next, threads, |s, part| {
                for (i, x) in part.iter_mut().enumerate() {
                    *x = p[p[s + i] as usize];
                }
            });
        }
        std::mem::swap(&mut ptr, &mut next);
    }
    let mut out = vec![NONE; n];
    let p = &ptr;
    par_fill(&mut out, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let t = p[s + i] as usize;
            *x = if p[t] as usize == t { val[t] } else { NONE };
        }
    });
    out
}

pub(crate) fn prepare_par(g: &Graph, threads: usize, times: &mut Vec<(&'static str, f64)>) -> Prep {
    let n = g.n;
    let mut t = cpu();
    let mut lap = |name: &'static str, times: &mut Vec<(&'static str, f64)>| {
        let now = cpu();
        times.push((name, now - t));
        t = now;
    };
    // 1. live: level-synchronous backward search over the reverse CSR; a node joins once
    //    (compare-and-swap on its flag), into the next level of the thread that claimed it.
    let (roff, rsrc) = reverse(g);
    let live_a: Vec<AtomicBool> = (0..n).map(|v| AtomicBool::new(g.sink[v].is_some())).collect();
    let mut frontier: Vec<u32> = (0..n as u32).filter(|&v| g.sink[v as usize].is_some()).collect();
    while !frontier.is_empty() {
        let c = frontier.len().div_ceil(threads).max(256);
        let parts: Vec<Vec<u32>> = std::thread::scope(|sc| {
            let hs: Vec<_> = frontier
                .chunks(c)
                .map(|part| {
                    let (live_a, roff, rsrc) = (&live_a, &roff, &rsrc);
                    sc.spawn(move || {
                        let mut out = Vec::new();
                        for &v in part {
                            for &u in &rsrc[roff[v as usize]..roff[v as usize + 1]] {
                                let f = &live_a[u as usize];
                                if !f.load(Ordering::Relaxed) && !f.swap(true, Ordering::Relaxed) {
                                    out.push(u);
                                }
                            }
                        }
                        out
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        frontier = parts.concat();
    }
    let live: Vec<bool> = live_a.into_iter().map(AtomicBool::into_inner).collect();
    drop((roff, rsrc));
    lap("live", times);
    // 2. next: one independent scan per node.
    let mut nxt = vec![NONE; n];
    par_fill(&mut nxt, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let u = s + i;
            for e in g.out(u as u32) {
                let d = g.dst[e] as usize;
                if live[d] && (d != u || g.sink[d].is_some()) {
                    *x = e as u32;
                    break;
                }
            }
        }
    });
    lap("next", times);
    // 3. first_of: a node whose next edge ends at a target, or that has none, is a terminal.
    let target_of = |v: usize| if nxt[v] == NONE { None } else { Some(g.dst[nxt[v] as usize] as usize) };
    let mut ptr = vec![0u32; n];
    let mut val = vec![NONE; n];
    par_fill(&mut ptr, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let v = s + i;
            *x = match target_of(v) {
                Some(d) if g.sink[d].is_none() => d as u32,
                _ => v as u32,
            };
        }
    });
    par_fill(&mut val, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            if let Some(d) = target_of(s + i).filter(|&d| g.sink[d].is_some()) {
                *x = d as u32;
            }
        }
    });
    let first = jump(ptr, &val, threads);
    lap("first_of", times);
    // 4. knot: sequential.
    let knot = knot_pass(g, &live);
    lap("knot", times);
    // 5. ent: a knot node is its own entry; a node outside the inner set, or whose chain leaves
    //    it, ends with NONE. Every next-loop lies in the knot, so no chain loops here.
    let inner = |u: usize| live[u] && g.sink[u].is_none();
    let mut ptr = vec![0u32; n];
    let mut val = vec![NONE; n];
    par_fill(&mut ptr, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let v = s + i;
            *x = match target_of(v) {
                Some(d) if inner(v) && !knot[v] && inner(d) => d as u32,
                _ => v as u32,
            };
        }
    });
    par_fill(&mut val, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let v = s + i;
            if inner(v) && knot[v] {
                *x = v as u32;
            }
        }
    });
    let ent = jump(ptr, &val, threads);
    lap("ent", times);
    Prep { live, nxt, first, knot, ent }
}

pub(crate) fn reverse(g: &Graph) -> (Vec<usize>, Vec<u32>) {
    let n = g.n;
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
    (roff, rsrc)
}

/// Answer the roots on several threads, giving `Run::run`'s answers.
///
/// Fresh memo (`dfs_first`): every answer is independent. The distinct entry nodes are searched
/// in parallel, each with its own memo, then every root is answered from them.
/// Shared memo (`dfs_first_memo`): the answer of a root that meets the knot depends on the roots
/// before it, so those run in root order on one thread. Pure roots never read the memo and are
/// answered in parallel.
pub(crate) fn run_par(g: &Graph, p: &Prep, roots: &[u32], node_mode: bool, share: bool, threads: usize) -> (Vec<Found>, u64) {
    let on_edge = !node_mode;
    // A root needs a search when it is a target in edge mode, or is live and meets the knot.
    let needs = |r: u32| {
        let ru = r as usize;
        if g.sink[ru].is_some() { on_edge } else { p.live[ru] && p.ent[ru] != NONE }
    };
    let pure = |r: u32| -> Found {
        let ru = r as usize;
        if let Some(l) = g.sink[ru] {
            Some((r, l))
        } else if !p.live[ru] {
            None
        } else {
            Some((p.first[ru], g.loc[p.nxt[ru] as usize]))
        }
    };
    let mut out: Vec<Found> = vec![None; roots.len()];
    let mut steps = 0u64;
    if share {
        par_fill(&mut out, threads, |s, part| {
            for (i, x) in part.iter_mut().enumerate() {
                let r = roots[s + i];
                if !needs(r) {
                    *x = pure(r);
                }
            }
        });
        let mut memo = HashMap::new();
        for (i, &r) in roots.iter().enumerate() {
            if !needs(r) {
                continue;
            }
            let ru = r as usize;
            out[i] = if g.sink[ru].is_some() {
                search(g, p, r, true, &mut memo, &mut steps)
            } else {
                let u = p.ent[ru];
                match search(g, p, u, on_edge, &mut memo, &mut steps) {
                    None => search(g, p, r, on_edge, &mut memo, &mut steps),
                    a if u == r => a,
                    a => a.map(|(s, _)| (s, g.loc[p.nxt[ru] as usize])),
                }
            };
        }
        return (out, steps);
    }
    // Fresh memo. Search keys: the entry node, or the root itself when it is an edge-mode target.
    let key = |r: u32| if g.sink[r as usize].is_some() { r } else { p.ent[r as usize] };
    let mut keys: Vec<u32> = roots.iter().copied().filter(|&r| needs(r)).map(key).collect();
    keys.sort_unstable();
    keys.dedup();
    let mut ans: Vec<(Found, u64)> = vec![(None, 0); keys.len()];
    {
        let keys = &keys;
        let c = keys.len().div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for (j, part) in ans.chunks_mut(c).enumerate() {
                sc.spawn(move || {
                    for (i, x) in part.iter_mut().enumerate() {
                        let mut st = 0;
                        let a = search(g, p, keys[j * c + i], on_edge, &mut HashMap::new(), &mut st);
                        *x = (a, st);
                    }
                });
            }
        });
    }
    steps += ans.iter().map(|a| a.1).sum::<u64>();
    let (keys, ans) = (&keys, &ans);
    par_fill(&mut out, threads, |s, part| {
        for (i, x) in part.iter_mut().enumerate() {
            let r = roots[s + i];
            *x = if !needs(r) {
                pure(r)
            } else {
                let u = key(r);
                let a = ans[keys.binary_search(&u).unwrap()].0;
                if u == r { a } else { a.map(|(s, _)| (s, g.loc[p.nxt[r as usize] as usize])) }
            };
        }
    });
    (out, steps)
}

pub(crate) fn same(a: &Prep, b: &Prep) -> bool {
    a.live == b.live && a.nxt == b.nxt && a.first == b.first && a.knot == b.knot && a.ent == b.ent
}
