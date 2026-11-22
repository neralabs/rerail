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
//! wave, decision 0095 §5, `lanes::wave`).

use rerail_blocks::vector::Vector;
use super::{Body, Component, JoinRule, Key, SEED, Src, Step, Store, Term, run_join_each};
use crate::code_rule_dsl::logic::lanes::prof;
use rerail_accel::{Bits, Kernel, Output};
use rerail_blocks::maps::BlockHash;

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
fn step_slots(s: &Step, out: &mut Vector<usize>) {
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
    let others: Vector<usize> = (0..k).filter(|&c| c != col).map(|c| slot_of[c]).collect();
    let rest = &j.steps[1..];
    let mut used = rerail_blocks::vector::Vector::new();
    for s in rest {
        if reads(s, 0) {
            return None;
        }
        step_slots(s, &mut used);
    }
    if used.iter().any(|s| others.contains(s)) || !used.contains(&z) {
        return None;
    }
    // The rest's steps made again, its run's objects the same ones (`steps::Plan::decoded`'s), as a join of its own.
    let names = j.names();
    let mut plan = super::steps::Plan {
        steps: rest.iter().map(|s| <super::steps::Step as rerail_section::Decode>::decode(s, &names)).collect(),
        out: vec![Term::Slot(z)],
        nslots: j.nslots,
        multi: Vec::new(),
        probes: Vec::new(),
        fused: (0..rest.len()).map(|_| None).collect(),
        spec: 0,
        from: (0..rest.len()).collect(),
        plan_steps: rest.len(),
        // The steps' ranges as they were: their ends are slots bound before them, the removed first step's too.
        ranges: j.ranges[1..].to_vec(),
    };
    let drafted = plan.encode();
    plan.probes = super::probes(&drafted.steps, &drafted.out);
    let rest = JoinRule::encoded(plan.encode());
    Some(Closure { col, rest, y })
}

/// What the closure did, for the run's report: nodes reached, rows made, and which way.
pub(super) struct Ran {
    pub(super) nodes: usize,
    pub(super) rows: usize,
    pub(super) how: String,
}

/// Where a node's reach comes from: row `local` of piece `piece`'s bit rows, or its component walked as a chain
/// (`CHAIN`) or searched on the host, past the limits (`HOST`). Plain words, a node's in its block.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct At {
    piece: u32,
    local: u32,
}

// SAFETY: `repr(C)`, two `u32`s.
unsafe impl rerail_blocks::Pod for At {}

impl At {
    const CHAIN: At = At { piece: u32::MAX, local: 0 };
    const HOST: At = At { piece: u32::MAX - 1, local: 0 };
    fn dense(piece: u32, local: u32) -> At {
        At { piece, local }
    }
    fn is_dense(self) -> bool {
        self.piece < u32::MAX - 1
    }
}

/// Lists of node ids in one array (a list's ids from `off[i]` to `off[i + 1]`): the reach graph's out-edges by node,
/// a piece's nodes, a component's members, each in its block.
#[derive(Default)]
struct Lists {
    off: Vector<u32>,
    ids: Vector<u32>,
}

impl Lists {
    fn new() -> Self {
        let mut off = Vector::new();
        off.push(0);
        Lists { off, ids: Vector::new() }
    }
    fn len(&self) -> usize {
        self.off.len().saturating_sub(1)
    }
    fn get(&self, i: usize) -> &[u32] {
        &self.ids[self.off[i] as usize..self.off[i + 1] as usize]
    }
    /// The list being made, ended (`end`) once its ids are pushed.
    fn push_id(&mut self, id: u32) {
        self.ids.push(id);
    }
    fn end(&mut self) {
        self.off.push(self.ids.len() as u32);
    }
    fn iter(&self) -> impl Iterator<Item = &[u32]> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }
}

/// The closure's first half done: the reach graph, where each node's reach comes from, and the launches the second
/// half needs (one per piece, in piece order).
pub(super) struct Prepared {
    nodes: Vector<i64>,
    index: BlockHash<i64, u32>,
    adj: Lists,
    /// Per node; empty when every component is a chain.
    at: Vector<At>,
    /// Each piece's nodes by local id.
    pieces: Lists,
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
    let mut index: BlockHash<i64, u32> = BlockHash::default();
    let mut nodes: Vector<i64> = Vector::new();
    for id in 0..rel.n {
        if rel.dead(id as u32) {
            continue;
        }
        let v = rel.value(id, cl.col);
        index.entry(v).or_insert_with(|| {
            nodes.push(v);
            (nodes.len() - 1) as u32
        });
    }
    let mut adj = Lists::new();
    let mut started = false;
    run_join_each(&cl.rest, store, syms, cl.y, &mut |out: &mut Vector<i64>| {
        // `out`: the edges out of node `adj.len()`, once a node was searched.
        if started {
            out.sort_unstable();
            out.dedup();
            for &z in out.iter() {
                let id = *index.entry(z).or_insert_with(|| {
                    nodes.push(z);
                    (nodes.len() - 1) as u32
                });
                adj.push_id(id);
            }
            adj.end();
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
        return Some(Prepared { nodes, index, adj, at: Vector::new(), pieces: Lists::new(), launches: Vec::new(), levels });
    }
    let _h = prof::host("closure", "encode: components, pieces, adjacency");
    let (at, pieces, launches) = partition(&adj);
    Some(Prepared { nodes, index, adj, at, pieces, launches, levels: false })
}

/// The reach graph's weakly connected components, each a chain, a part of a dense piece, or walked on the host; and
/// each piece's launch, its edges renumbered within it.
fn partition(adj: &Lists) -> (Vector<At>, Lists, Vec<Kernel>) {
    let n = adj.len();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            let up = p[p[x as usize] as usize];
            p[x as usize] = up;
            x = up;
        }
        x
    }
    let mut parent: Vector<u32> = (0..n as u32).collect();
    for (v, out) in adj.iter().enumerate() {
        for &w in out {
            let (a, b) = (find(&mut parent, v as u32), find(&mut parent, w));
            if a != b {
                parent[a.max(b) as usize] = a.min(b);
            }
        }
    }
    // Members of each component, ascending, components by their least node: each node's component, then the
    // components' members counted and placed.
    let mut comp_of: Vector<u32> = rerail_blocks::vector![u32::MAX; n];
    let mut of_node: Vector<u32> = rerail_blocks::vector![0; n];
    let mut count = 0u32;
    for v in 0..n as u32 {
        let r = find(&mut parent, v) as usize;
        if comp_of[r] == u32::MAX {
            comp_of[r] = count;
            count += 1;
        }
        of_node[v as usize] = comp_of[r];
    }
    let mut comps = Lists { off: rerail_blocks::vector![0; count as usize + 1], ids: rerail_blocks::vector![0; n] };
    for &c in of_node.iter() {
        comps.off[c as usize + 1] += 1;
    }
    for c in 0..count as usize {
        comps.off[c + 1] += comps.off[c];
    }
    let mut fill: Vector<u32> = Vector::from(&comps.off[..count as usize]);
    for v in 0..n as u32 {
        let c = of_node[v as usize] as usize;
        comps.ids[fill[c] as usize] = v;
        fill[c] += 1;
    }
    let mut at: Vector<At> = rerail_blocks::vector![At::CHAIN; n];
    // Dense components packed into pieces, in order: a piece's nodes end where its next begins.
    let mut packed = Lists::new();
    let (mut chains, mut host, mut host_nodes) = (0u64, 0u64, 0u64);
    let mut cur = 0usize;
    for c in comps.iter() {
        if c.iter().all(|&v| adj.get(v as usize).len() <= 1) {
            chains += 1;
            continue;
        }
        if c.len() > MAX_DENSE {
            c.iter().for_each(|&v| at[v as usize] = At::HOST);
            host += 1;
            host_nodes += c.len() as u64;
            continue;
        }
        if cur != 0 && cur + c.len() > PACK {
            packed.end();
            cur = 0;
        }
        // copy: derived: a component's nodes (the closure's own answer), packed into its piece.
        packed.ids.extend_from_slice(c);
        cur += c.len();
    }
    if cur != 0 {
        packed.end();
    }
    // The bits budget: the largest pieces go to the host until the rest fit.
    let bits = |p: &[u32]| (p.len() as u64).pow(2);
    let mut total: u64 = packed.iter().map(bits).sum();
    let mut gone: Vector<u8> = rerail_blocks::vector![0; packed.len()];
    if total > MAX_BITS {
        let mut order: Vector<usize> = (0..packed.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(packed.get(i).len()));
        for i in order {
            if total <= MAX_BITS {
                break;
            }
            total -= bits(packed.get(i));
            gone[i] = 1;
            packed.get(i).iter().for_each(|&v| at[v as usize] = At::HOST);
            host += 1;
            host_nodes += packed.get(i).len() as u64;
        }
    }
    let mut pieces = Lists::new();
    for i in (0..packed.len()).filter(|&i| gone[i] == 0) {
        // copy: derived: the pieces kept for the device, of the components above.
        pieces.ids.extend_from_slice(packed.get(i));
        pieces.end();
    }
    let mut launches = Vec::with_capacity(pieces.len());
    for (pi, p) in pieces.iter().enumerate() {
        for (local, &v) in p.iter().enumerate() {
            at[v as usize] = At::dense(pi as u32, local as u32);
        }
        let mut off = Vector::with_capacity(p.len() + 1);
        off.push(0u32);
        let mut dst = Vector::new();
        for &v in p {
            dst.extend(adj.get(v as usize).iter().map(|&w| {
                let a = at[w as usize];
                assert!(a.is_dense(), "an edge leaves its component");
                a.local
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
    // A host search's reach, once per start: its nodes' place in `found` (start, length).
    let mut searched: BlockHash<u32, [u32; 2]> = BlockHash::default();
    let mut found: Vector<u32> = Vector::new();
    let mut seen: Vector<u32> = Vector::new();
    let mut stack: Vector<u32> = Vector::new();
    let mut epoch = 0u32;
    let mut reach = |s: u32, into: &mut Vector<u32>| {
        let a = p.at.get(s as usize).copied().unwrap_or(At::CHAIN);
        if a == At::CHAIN {
            chain = true;
            // Walk until the chain ends or comes back (a cycle: every node on it once).
            let mut v = s;
            while let Some(&w) = p.adj.get(v as usize).first() {
                if into.contains(&w) {
                    break;
                }
                into.push(w);
                if w == s {
                    break;
                }
                v = w;
            }
        } else if a.is_dense() {
            dense = true;
            let nodes = p.pieces.get(a.piece as usize);
            for (w, word) in bits[a.piece as usize].row(a.local as usize).iter().enumerate() {
                let mut word = *word;
                while word != 0 {
                    into.push(nodes[w * 64 + word.trailing_zeros() as usize]);
                    word &= word - 1;
                }
            }
        } else {
            host = true;
            let [from, len] = *searched.get_or_insert_with(s, || {
                if seen.is_empty() {
                    seen = rerail_blocks::vector![0; n];
                }
                epoch += 1;
                let from = found.len();
                stack.clear();
                stack.extend(p.adj.get(s as usize).iter().copied());
                while let Some(w) = stack.pop() {
                    if seen[w as usize] == epoch {
                        continue;
                    }
                    seen[w as usize] = epoch;
                    found.push(w);
                    stack.extend(p.adj.get(w as usize).iter().copied().filter(|&x| seen[x as usize] != epoch));
                }
                [from as u32, (found.len() - from) as u32]
            });
            // copy: derived: a node's reach, found just above, written to its row of the answer.
            into.extend_from_slice(&found[from as usize..(from + len) as usize]);
        }
    };
    let _h = prof::host("closure", if p.at.is_empty() { "decode: chain walks, rows into the relation" } else { "decode: reach bits and walks, rows into the relation" });
    let rel = &mut store.rels[0];
    let seeds = rel.n;
    let mut row: Vector<i64> = rerail_blocks::vector![0; rel.k];
    let mut into = Vector::new();
    // Each row pushed once (the relation's push is its set check): no buffer of fresh rows to hash first.
    let mut rows = 0;
    for id in 0..seeds {
        if rel.dead(id as u32) {
            continue;
        }
        row.copy_from_slice(&rel.row(id));
        into.clear();
        reach(*p.index.get(&row[cl.col]).expect("a seed's node"), &mut into);
        for &z in &into {
            row[cl.col] = p.nodes[z as usize];
            rows += rel.push(&row) as usize;
        }
    }
    prof::count("closure", "rows made", rows as u64);
    let how: Vec<&str> = [(dense, "bit rows"), (chain, "chains"), (host, "host walks past the limits")].iter().filter(|w| w.0).map(|w| w.1).collect();
    Some(Ran { nodes: n, rows, how: if how.is_empty() { "chains".into() } else { how.join(" and ") } })
}

/// The closure's rows with their levels: from each seed row, a search along `adj` gives each node its distance, and a
/// row's level is the least of its seeds' levels plus their distances.
fn by_distance(cl: &Closure, rel: &mut super::Rel, index: &BlockHash<i64, u32>, nodes: &[i64], adj: &Lists) -> Ran {
    let seeds = rel.n;
    let k = rel.k;
    // A start node's reach with distances, searched once per distinct start: its (node, distance) pairs' place in
    // `found` (start, length).
    let mut reach_of: BlockHash<u32, [u32; 2]> = BlockHash::default();
    let mut found: Vector<[u32; 2]> = Vector::new();
    let mut seen: Vector<u32> = rerail_blocks::vector![u32::MAX; nodes.len()];
    let mut stamp = 0u32;
    let (mut frontier, mut next): (Vector<u32>, Vector<u32>) = (Vector::new(), Vector::new());
    let mut search = |s: u32, found: &mut Vector<[u32; 2]>| -> [u32; 2] {
        stamp += 1;
        let from = found.len();
        frontier.clear();
        frontier.push(s);
        let mut d = 0;
        while !frontier.is_empty() {
            d += 1;
            next.clear();
            for &v in frontier.iter() {
                for &w in adj.get(v as usize) {
                    if seen[w as usize] != stamp {
                        seen[w as usize] = stamp;
                        found.push([w, d]);
                        next.push(w);
                    }
                }
            }
            std::mem::swap(&mut frontier, &mut next);
        }
        [from as u32, (found.len() - from) as u32]
    };
    // The rows made, `k` values each, a set over them (`RowSet`), and each one's least level.
    let mut made: Vector<i64> = Vector::new();
    let mut levels: Vector<u32> = Vector::new();
    let mut set = super::keys::RowSet::default();
    let mut row: Vector<i64> = rerail_blocks::vector![0i64; k];
    for id in 0..seeds {
        if rel.dead(id as u32) {
            continue;
        }
        row.copy_from_slice(&rel.row(id));
        let base = rel.level[id];
        let s = *index.get(&row[cl.col]).expect("a seed's node");
        let [from, len] = match reach_of.get(&s) {
            Some(&r) => r,
            None => {
                let r = search(s, &mut found);
                reach_of.insert(s, r);
                r
            }
        };
        for j in from as usize..(from + len) as usize {
            let [z, d] = found[j];
            row[cl.col] = nodes[z as usize];
            let level = base + d;
            match set.find(&made, k, &row, |_| false) {
                Some(i) => levels[i as usize] = levels[i as usize].min(level),
                None => {
                    // copy: derived: a head row the closure makes, written once into its relation.
                    made.extend_from_slice(&row);
                    levels.push(level);
                    set.insert(&made, k, (levels.len() - 1) as u32);
                }
            }
        }
    }
    let mut rows = 0;
    for (i, level) in levels.iter().enumerate() {
        if rel.push(&made[i * k..(i + 1) * k]) {
            *rel.level.last_mut().expect("a level pushed") = *level;
            rows += 1;
        }
    }
    Ran { nodes: nodes.len(), rows, how: "levels by distance".into() }
}
