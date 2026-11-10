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
//! The reach (`E⁺`):
//! - when every node has at most one successor (a climb up a tree, or any functional edge), by walking each row's
//!   chain, which costs what it emits;
//! - otherwise, when the nodes are few enough for bit rows (`MAX_DENSE`), by the executor's `Closure` kernel (SCC
//!   condensation and bit rows), placed on a lane by its price;
//! - otherwise the component runs semi-naive from the seeds, as it would have (the planner's other form).

use super::{Body, Component, Fresh, JoinRule, Key, SEED, Src, Step, Store, Term, run_join_collect};
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

/// The most nodes for bit rows: 16,384² bits are 32 MB.
const MAX_DENSE: usize = 16_384;

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

/// The component's closure, when it has the shape (see the module's comment). `RERAIL_CLOSURE_FORM=off` turns it off.
pub(super) fn recognize(c: &Component) -> Option<Closure> {
    if std::env::var("RERAIL_CLOSURE_FORM").is_ok_and(|v| v == "off") {
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
    };
    Some(Closure { col, rest, y })
}

/// What the closure did, for the run's report: nodes reached, rows made, and which way.
pub(super) struct Ran {
    pub(super) nodes: usize,
    pub(super) rows: usize,
    pub(super) how: &'static str,
}

/// Runs the closure over relation 0's rows (the seeds, already in it). `None` when the reach is too large for bit rows
/// and not a chain: the caller then runs semi-naive from the seeds.
pub(super) fn run(cl: &Closure, store: &mut Store, syms: &mut super::Interner, lanes: &rerail_accel::Executor) -> Option<Ran> {
    // The distinct start nodes, then the edges out of every node reached, by search.
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
    let mut at = 0;
    let mut out = Vec::new();
    while at < nodes.len() {
        out.clear();
        // An error is the semi-naive path's to report: it runs the same steps.
        run_join_collect(&cl.rest, store, syms, &[(cl.y, nodes[at])], &mut out).ok()?;
        out.sort_unstable();
        out.dedup();
        let mut next = Vec::with_capacity(out.len());
        for &z in &out {
            let id = *index.entry(z).or_insert_with(|| {
                nodes.push(z);
                (nodes.len() - 1) as u32
            });
            next.push(id);
        }
        adj.push(next);
        at += 1;
    }
    let n = nodes.len();
    // Each row's level (decision 0090), when the relation keeps them: a row is made from the one a hop before it,
    // so its level is its seed's plus its distance (the least over its seeds), which puts the row it is made from
    // strictly below it. One push of them all would put every row at one level.
    if store.rels[0].level.len() == store.rels[0].n {
        return Some(by_distance(cl, &mut store.rels[0], &index, &nodes, &adj));
    }
    let chain = adj.iter().all(|a| a.len() <= 1);
    // Each start node's reach, as node ids.
    let reach: Box<dyn Fn(u32, &mut Vec<u32>)> = if chain {
        let adj = Arc::new(adj);
        Box::new(move |s: u32, into: &mut Vec<u32>| {
            // Walk until the chain ends or comes back (a cycle: every node on it once).
            let mut v = s;
            while let Some(&w) = adj[v as usize].first() {
                if into.contains(&w) {
                    break;
                }
                into.push(w);
                if w == s {
                    break;
                }
                v = w;
            }
        })
    } else if n <= MAX_DENSE {
        let mut off = vec![0u32];
        let mut dst = Vec::new();
        for a in &adj {
            dst.extend_from_slice(a);
            off.push(dst.len() as u32);
        }
        let k = rerail_accel::Kernel::Closure { n: n as u32, off: off.into(), dst: dst.into() };
        let (mut done, _) = lanes.run(rerail_accel::Batch(vec![rerail_accel::Launch::new(k)])).ok()?;
        let rerail_accel::Output::Bits(bits) = done.remove(0).out else { return None };
        Box::new(move |s: u32, into: &mut Vec<u32>| {
            for (w, word) in bits.row(s as usize).iter().enumerate() {
                let mut word = *word;
                while word != 0 {
                    into.push((w * 64) as u32 + word.trailing_zeros());
                    word &= word - 1;
                }
            }
        })
    } else {
        return None;
    };
    // The rows: each seed row with its column replaced by every node of its start's reach.
    let rel = &mut store.rels[0];
    let seeds = rel.n;
    let mut fresh = Fresh::default();
    let mut row = vec![0i64; rel.k];
    let mut into = Vec::new();
    for id in 0..seeds {
        if rel.dead(id as u32) {
            continue;
        }
        row.copy_from_slice(rel.row(id));
        into.clear();
        reach(index[&row[cl.col]], &mut into);
        for &z in &into {
            row[cl.col] = nodes[z as usize];
            fresh.add(&row);
        }
    }
    let mut rows = 0;
    for i in 0..fresh.n {
        rows += rel.push(&fresh.data[i * rel.k..][..rel.k]) as usize;
    }
    Some(Ran { nodes: n, rows, how: if chain { "chains" } else { "bit rows" } })
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
    Ran { nodes: nodes.len(), rows, how: "levels by distance" }
}
