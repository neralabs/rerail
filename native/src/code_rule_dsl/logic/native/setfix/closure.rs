//! The `closure` form (decision 0098 §1): a linear recursion that walks one column along an edge, run as the edge's
//! closure over distinct nodes instead of semi-naive rounds over rows.
//!
//! The shape. A component of one relation `r`, whose seeds are native joins, and whose one recursive probe reads
//! `r`'s delta with a distinct variable in every column and makes the same row with one column `y` replaced:
//!
//! ```text
//! r(x1, …, y, …, xk) :- seeds…
//! r(x1, …, z, …, xk) :- r(x1, …, y, …, xk), rest(y, z).      // rest names only y, z and variables of its own
//! ```
//!
//! Then `r = seeds ∪ { row[y := z] : row ∈ seeds, z ∈ E⁺(row[y]) }`, where `E(y) = { z : rest(y, z) }`. Semi-naive
//! runs `rest` (its joins, negations, tests) once per row at every step; here it runs once per **distinct node** the
//! walk reaches, and the rows are emitted from each node's reach. Many rows climb through the same nodes (every
//! descendant of a scope walks through the scope's ancestors), so this does the rest's work once per node.
//!
//! The reach (`E⁺`). No path leaves a weakly connected component of the reach graph, so each component is its own
//! problem:
//! - when every node has at most one successor (a climb up a tree, or any functional edge), by walking each row's
//!   chain, which costs what it emits; when every component is a chain, nothing else is built;
//! - otherwise the components of at most `MAX_DENSE` nodes are packed into **pieces** of about `PACK` nodes, and each
//!   piece is one `Closure` launch of the executor (SCC condensation and bit rows), placed on a lane by its price.
//!   A piece's bits grow with its nodes squared, so packing keeps them at about the graph's nodes times `PACK`;
//! - a component past `MAX_DENSE` nodes (or past the bits budget, `MAX_BITS`) is walked on the host: a search from
//!   each start node in it, once per start. That reads each start's reach once, which semi-naive reads once per row.
//!
//! The form runs in two halves around its launches (`prepare`, `finish`), so a stratum can wait for them while the
//! other strata of its level prepare theirs: the launches of a level then go to the executor as one batch (a device
//! wave, decision 0095b §5, `lanes::wave`).

use super::{Body, Component, JoinRule, Key, SEED, Src, Step, Store, Term, run_join_each};
use crate::code_rule_dsl::logic::lanes::prof;
use rerail_accel::{Bits, Kernel, Output};
use rustc_hash::FxHashMap as HashMap;

/// The most nodes of one launch's bit rows: 16,384² bits are 32 MB.
pub const MAX_DENSE: usize = 16_384;
/// Components are packed into pieces of about this many nodes (a larger component is a piece alone).
const PACK: usize = 1_024;
/// The most bits a closure's launches hold at once (128 MB): past it, the largest pieces are walked on the host.
const MAX_BITS: u64 = 1 << 30;

/// A recognized closure: the walking column, the probe's rest as a join from `y` to `z`, and the slot of `y`.
pub(super) struct Closure {
    col: usize,
    rest: JoinRule,
    y: usize,
}

/// Every slot a step reads or binds.
fn step_slots(s: &Step, out: &mut Vec<usize>) {
    let mut keys: Vec<Key> = Vec::new();
    match s {
        Step::Atom { keys: ks, binds, .. } => {
            keys.extend(ks.iter().map(|(_, x)| *x));
            out.extend(binds.iter().map(|&(_, s)| s));
        }
        Step::Scalar { keys: ks, out: o, .. } | Step::StrJoin { keys: ks, out: o, .. } => {
            keys.extend(ks.iter().map(|(_, x)| *x));
            out.push(*o);
        }
        Step::Cat { out: o, a, b, .. } => {
            keys.extend([*a, *b]);
            out.push(*o);
        }
        Step::Split { whole, a, b, ka, kb } => {
            keys.extend([*whole, *ka, *kb]);
            out.extend(a.iter().chain(b.iter()).copied());
        }
        Step::Eq { a, b } | Step::Ne { a, b } | Step::Lt { a, b, .. } | Step::Doc { a, b, .. } => keys.extend([*a, *b]),
        Step::Let { a, b } => {
            out.push(*a);
            keys.push(*b);
        }
        Step::Func { args, out: o, .. } => {
            keys.extend(args.iter().copied());
            out.push(*o);
        }
        Step::DocOf { out: o, n, .. } => {
            keys.push(*n);
            out.push(*o);
        }
        Step::Test { s, p, .. } => keys.extend([*s, *p]),
        Step::In { a, .. } => keys.push(*a),
        Step::Unused { .. } => {}
    }
    out.extend(keys.into_iter().filter_map(|k| if let Key::Slot(s) = k { Some(s) } else { None }));
}

/// Whether a step reads the component's relation `r` (a second recursive read).
fn reads(s: &Step, r: usize) -> bool {
    let src = match s {
        Step::Atom { src, .. } | Step::Scalar { src, .. } | Step::StrJoin { src, .. } => src,
        _ => return false,
    };
    matches!(src, Src::Delta(x) | Src::Full(x) | Src::FullMulti(x, _) if *x == r)
}

thread_local! {
    /// Tests: this thread's components run semi-naive, for a reference (`semi_naive_here`).
    static SEMI_NAIVE_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: components opened on this thread run semi-naive (`on`) or may take the form.
#[cfg(any(test, feature = "test-hooks"))]
pub fn semi_naive_here(on: bool) {
    SEMI_NAIVE_HERE.set(on);
}

/// The component's closure, when it has the shape (see the module's comment). `RERAIL_CLOSURE_FORM=off` turns it off.
pub(super) fn recognize(c: &Component) -> Option<Closure> {
    if std::env::var("RERAIL_CLOSURE_FORM").is_ok_and(|v| v == "off") || SEMI_NAIVE_HERE.get() {
        return None;
    }
    let store = c.store.read().unwrap();
    if store.rels.len() != 1 || !c.edges.is_empty() || !c.gates.is_empty() || !c.requests.is_empty() || !c.produced.is_empty() {
        return None;
    }
    let k = store.rels[0].k;
    drop(store);
    let mut probe = None;
    for p in &c.probes {
        match (&p.body, p.delta) {
            (Body::Join(_), SEED) => {}
            (Body::Join(j), 0) if p.head == 0 && probe.is_none() => probe = Some(j),
            _ => return None,
        }
    }
    let j = probe?;
    // The delta atom first, a distinct slot in every column.
    let Some(Step::Atom { src: Src::Delta(0), neg: false, keys, binds, tests }) = j.steps.first() else { return None };
    if !keys.is_empty() || !tests.is_empty() || binds.len() != k {
        return None;
    }
    let mut slot_of = vec![usize::MAX; k];
    for &(c, s) in binds {
        slot_of[c] = s;
    }
    if slot_of.contains(&usize::MAX) || (1..k).any(|i| slot_of[..i].contains(&slot_of[i])) {
        return None;
    }
    // The head: the same slots, but one column's, which is a slot of the rest.
    if j.out.len() != k {
        return None;
    }
    let mut col = None;
    for (c, o) in j.out.iter().enumerate() {
        let Term::Slot(s) = o else { return None };
        if *s != slot_of[c] {
            if col.is_some() || slot_of.contains(s) {
                return None;
            }
            col = Some((c, *s));
        }
    }
    let (col, z) = col?;
    let y = slot_of[col];
    // The rest names no other column's slot, and reads `r` nowhere.
    let others: Vec<usize> = (0..k).filter(|&c| c != col).map(|c| slot_of[c]).collect();
    let rest = &j.steps[1..];
    let mut used = Vec::new();
    for s in rest {
        if reads(s, 0) {
            return None;
        }
        step_slots(s, &mut used);
    }
    if used.iter().any(|s| others.contains(s)) || !used.contains(&z) {
        return None;
    }
    let rest = JoinRule {
        steps: rest.iter().map(|s| s.clone()).collect(),
        out: vec![Term::Slot(z)],
        nslots: j.nslots,
        multi: Vec::new(),
        counts: (0..rest.len()).map(|_| Default::default()).collect(),
        fused: (0..rest.len()).map(|_| None).collect(),
        forms: (0..rest.len()).map(|_| Default::default()).collect(),
        spec: 0,
        passed_over: Default::default(),
    };
    Some(Closure { col, rest, y })
}

/// What the closure did, for the run's report: nodes reached, rows made, and which way.
pub(super) struct Ran {
    pub(super) nodes: usize,
    pub(super) rows: usize,
    pub(super) how: String,
}

/// Where a node's reach comes from.
#[derive(Clone, Copy)]
enum At {
    /// Its component is a chain: walk it.
    Chain,
    /// Row `local` of piece `piece`'s bit rows.
    Dense { piece: u32, local: u32 },
    /// Its component is past the limits: a search on the host.
    Host,
}

/// The closure's first half done: the reach graph, where each node's reach comes from, and the launches the second
/// half needs (one per piece, in piece order).
pub(super) struct Prepared {
    nodes: Vec<i64>,
    index: HashMap<i64, u32>,
    adj: Vec<Vec<u32>>,
    /// Per node; empty when every component is a chain.
    at: Vec<At>,
    /// Each piece's nodes by local id.
    pieces: Vec<Vec<u32>>,
    pub(super) launches: Vec<Kernel>,
    /// The relation keeps levels (decision 0090): its rows are made by distance on the host (`by_distance`), with no
    /// launches.
    levels: bool,
}

/// The first half: the distinct start nodes, the edges out of every node reached (the rest, searched once per
/// node), then the reach graph's components, and the pieces' launches. `None` when the search fails: the caller then
/// runs semi-naive from the seeds.
pub(super) fn prepare(cl: &Closure, store: &mut Store, syms: &mut super::Interner) -> Option<Prepared> {
    let h = prof::host("closure", "encode: the reach's edges (the rest, per node)");
    let rel = &store.rels[0];
    let mut index: HashMap<i64, u32> = HashMap::default();
    let mut nodes: Vec<i64> = Vec::new();
    for id in 0..rel.n {
        if rel.dead(id as u32) {
            continue;
        }
        let v = rel.row(id)[cl.col];
        index.entry(v).or_insert_with(|| {
            nodes.push(v);
            (nodes.len() - 1) as u32
        });
    }
    let mut adj: Vec<Vec<u32>> = Vec::new();
    let mut started = false;
    run_join_each(&cl.rest, store, syms, cl.y, &mut |out: &mut Vec<i64>| {
        // `out`: the edges out of node `adj.len()`, once a node was searched.
        if started {
            out.sort_unstable();
            out.dedup();
            let mut next = Vec::with_capacity(out.len());
            for &z in out.iter() {
                let id = *index.entry(z).or_insert_with(|| {
                    nodes.push(z);
                    (nodes.len() - 1) as u32
                });
                next.push(id);
            }
            adj.push(next);
        }
        started = true;
        nodes.get(adj.len()).copied()
    })
    .ok()?;
    drop(h);
    let n = nodes.len();
    prof::count("closure", "nodes searched", n as u64);
    // Each row's level (decision 0090), when the relation keeps them: a row is made from the one a hop before it,
    // so its level is its seed's plus its distance (the least over its seeds), which puts the row it is made from
    // strictly below it. One push of them all would put every row at one level. Bit rows give reach, not distance.
    let levels = store.rels[0].level.len() == store.rels[0].n;
    if levels || adj.iter().all(|a| a.len() <= 1) {
        if !levels {
            prof::count("closure", "closures by chains alone", 1);
        }
        return Some(Prepared { nodes, index, adj, at: Vec::new(), pieces: Vec::new(), launches: Vec::new(), levels });
    }
    let _h = prof::host("closure", "encode: components, pieces, adjacency");
    let (at, pieces, launches) = partition(&adj);
    Some(Prepared { nodes, index, adj, at, pieces, launches, levels: false })
}

/// The reach graph's weakly connected components, each a chain, a part of a dense piece, or walked on the host; and
/// each piece's launch, its edges renumbered within it.
fn partition(adj: &[Vec<u32>]) -> (Vec<At>, Vec<Vec<u32>>, Vec<Kernel>) {
    let n = adj.len();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            let up = p[p[x as usize] as usize];
            p[x as usize] = up;
            x = up;
        }
        x
    }
    let mut parent: Vec<u32> = (0..n as u32).collect();
    for (v, out) in adj.iter().enumerate() {
        for &w in out {
            let (a, b) = (find(&mut parent, v as u32), find(&mut parent, w));
            if a != b {
                parent[a.max(b) as usize] = a.min(b);
            }
        }
    }
    // Members of each component, ascending, components by their least node.
    let mut comp_of = vec![u32::MAX; n];
    let mut comps: Vec<Vec<u32>> = Vec::new();
    for v in 0..n as u32 {
        let r = find(&mut parent, v) as usize;
        if comp_of[r] == u32::MAX {
            comp_of[r] = comps.len() as u32;
            comps.push(Vec::new());
        }
        comps[comp_of[r] as usize].push(v);
    }
    let mut at = vec![At::Chain; n];
    // Dense components packed into pieces, in order.
    let mut pieces: Vec<Vec<u32>> = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let (mut chains, mut host, mut host_nodes) = (0u64, 0u64, 0u64);
    for c in &comps {
        if c.iter().all(|&v| adj[v as usize].len() <= 1) {
            chains += 1;
            continue;
        }
        if c.len() > MAX_DENSE {
            c.iter().for_each(|&v| at[v as usize] = At::Host);
            host += 1;
            host_nodes += c.len() as u64;
            continue;
        }
        if !cur.is_empty() && cur.len() + c.len() > PACK {
            pieces.push(std::mem::take(&mut cur));
        }
        cur.extend_from_slice(c);
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    // The bits budget: the largest pieces go to the host until the rest fit.
    let bits = |p: &Vec<u32>| (p.len() as u64).pow(2);
    let mut total: u64 = pieces.iter().map(bits).sum();
    if total > MAX_BITS {
        let mut order: Vec<usize> = (0..pieces.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(pieces[i].len()));
        let mut gone = vec![false; pieces.len()];
        for i in order {
            if total <= MAX_BITS {
                break;
            }
            total -= bits(&pieces[i]);
            gone[i] = true;
            pieces[i].iter().for_each(|&v| at[v as usize] = At::Host);
            host += 1;
            host_nodes += pieces[i].len() as u64;
        }
        let mut k = 0;
        pieces.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
    }
    let mut launches = Vec::with_capacity(pieces.len());
    for (pi, p) in pieces.iter().enumerate() {
        for (local, &v) in p.iter().enumerate() {
            at[v as usize] = At::Dense { piece: pi as u32, local: local as u32 };
        }
        let mut off = Vec::with_capacity(p.len() + 1);
        off.push(0u32);
        let mut dst = Vec::new();
        for &v in p {
            dst.extend(adj[v as usize].iter().map(|&w| match at[w as usize] {
                At::Dense { local, .. } => local,
                _ => unreachable!("an edge leaves its component"),
            }));
            off.push(dst.len() as u32);
        }
        launches.push(Kernel::Closure { n: p.len() as u32, off: off.into(), dst: dst.into() });
    }
    prof::count("closure", "components walked as chains", chains);
    prof::count("closure", "pieces by bit rows (launches)", pieces.len() as u64);
    prof::count("closure", "components walked on the host (past the limits)", host);
    prof::count("closure", "nodes walked on the host (past the limits)", host_nodes);
    (at, pieces, launches)
}

/// The second half: each seed row with its column replaced by every node of its start's reach, from the chains, the
/// pieces' bit rows (`outs`, one per launch) and the host's searches.
pub(super) fn finish(cl: &Closure, p: Prepared, outs: Vec<Output>, store: &mut Store) -> Option<Ran> {
    if p.levels {
        return Some(by_distance(cl, &mut store.rels[0], &p.index, &p.nodes, &p.adj));
    }
    let bits: Vec<Bits> = outs.into_iter().map(|o| if let Output::Bits(b) = o { Some(b) } else { None }).collect::<Option<_>>()?;
    if bits.len() != p.pieces.len() {
        return None;
    }
    let n = p.nodes.len();
    let (mut dense, mut chain, mut host) = (false, false, false);
    // A host search's reach, once per start.
    let mut searched: HashMap<u32, Vec<u32>> = HashMap::default();
    let mut seen: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut epoch = 0u32;
    let mut reach = |s: u32, into: &mut Vec<u32>| match p.at.get(s as usize).copied().unwrap_or(At::Chain) {
        At::Chain => {
            chain = true;
            // Walk until the chain ends or comes back (a cycle: every node on it once).
            let mut v = s;
            while let Some(&w) = p.adj[v as usize].first() {
                if into.contains(&w) {
                    break;
                }
                into.push(w);
                if w == s {
                    break;
                }
                v = w;
            }
        }
        At::Dense { piece, local } => {
            dense = true;
            let nodes = &p.pieces[piece as usize];
            for (w, word) in bits[piece as usize].row(local as usize).iter().enumerate() {
                let mut word = *word;
                while word != 0 {
                    into.push(nodes[w * 64 + word.trailing_zeros() as usize]);
                    word &= word - 1;
                }
            }
        }
        At::Host => {
            host = true;
            let r = searched.entry(s).or_insert_with(|| {
                if seen.is_empty() {
                    seen = vec![0; n];
                }
                epoch += 1;
                let mut r = Vec::new();
                stack.clear();
                stack.extend(p.adj[s as usize].iter().copied());
                while let Some(w) = stack.pop() {
                    if seen[w as usize] == epoch {
                        continue;
                    }
                    seen[w as usize] = epoch;
                    r.push(w);
                    stack.extend(p.adj[w as usize].iter().copied().filter(|&x| seen[x as usize] != epoch));
                }
                r
            });
            into.extend_from_slice(r);
        }
    };
    let _h = prof::host("closure", if p.at.is_empty() { "decode: chain walks, rows into the relation" } else { "decode: reach bits and walks, rows into the relation" });
    let rel = &mut store.rels[0];
    let seeds = rel.n;
    let mut row = vec![0i64; rel.k];
    let mut into = Vec::new();
    // Each row pushed once (the relation's push is its set check): no buffer of fresh rows to hash first.
    let mut rows = 0;
    for id in 0..seeds {
        if rel.dead(id as u32) {
            continue;
        }
        row.copy_from_slice(rel.row(id));
        into.clear();
        reach(p.index[&row[cl.col]], &mut into);
        for &z in &into {
            row[cl.col] = p.nodes[z as usize];
            rows += rel.push(&row) as usize;
        }
    }
    drop(reach);
    prof::count("closure", "rows made", rows as u64);
    let how: Vec<&str> = [(dense, "bit rows"), (chain, "chains"), (host, "host walks past the limits")].iter().filter(|w| w.0).map(|w| w.1).collect();
    Some(Ran { nodes: n, rows, how: if how.is_empty() { "chains".into() } else { how.join(" and ") } })
}

/// The closure's rows with their levels: from each seed row, a search along `adj` gives each node its distance, and a
/// row's level is the least of its seeds' levels plus their distances.
fn by_distance(cl: &Closure, rel: &mut super::Rel, index: &HashMap<i64, u32>, nodes: &[i64], adj: &[Vec<u32>]) -> Ran {
    let seeds = rel.n;
    // A start node's reach with distances, searched once per distinct start.
    let mut reach_of: HashMap<u32, Vec<(u32, u32)>> = HashMap::default();
    let mut seen: Vec<u32> = vec![u32::MAX; nodes.len()];
    let mut stamp = 0u32;
    let mut search = |s: u32| -> Vec<(u32, u32)> {
        stamp += 1;
        let mut out: Vec<(u32, u32)> = Vec::new();
        let mut frontier = vec![s];
        let mut d = 0;
        while !frontier.is_empty() {
            d += 1;
            let mut next = Vec::new();
            for &v in &frontier {
                for &w in &adj[v as usize] {
                    if seen[w as usize] != stamp {
                        seen[w as usize] = stamp;
                        out.push((w, d));
                        next.push(w);
                    }
                }
            }
            frontier = next;
        }
        out
    };
    let mut at: HashMap<Vec<i64>, usize> = HashMap::default();
    let mut made: Vec<(Vec<i64>, u32)> = Vec::new();
    let mut row = vec![0i64; rel.k];
    for id in 0..seeds {
        if rel.dead(id as u32) {
            continue;
        }
        row.copy_from_slice(rel.row(id));
        let base = rel.level[id];
        let s = index[&row[cl.col]];
        let reach = reach_of.entry(s).or_insert_with(|| search(s));
        for &(z, d) in reach.iter() {
            row[cl.col] = nodes[z as usize];
            let level = base + d;
            match at.get(&row) {
                Some(&i) => made[i].1 = made[i].1.min(level),
                None => {
                    at.insert(row.clone(), made.len());
                    made.push((row.clone(), level));
                }
            }
        }
    }
    let mut rows = 0;
    for (r, level) in &made {
        if rel.push(r) {
            *rel.level.last_mut().expect("a level pushed") = *level;
            rows += 1;
        }
    }
    Ran { nodes: nodes.len(), rows, how: "levels by distance".into() }
}
