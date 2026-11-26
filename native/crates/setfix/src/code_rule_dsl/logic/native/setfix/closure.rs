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
//! **The keyed form (decision 0570).** A walk whose rest also reads the row's other columns (`!bound_in(s, x)`: the
//! scope a name is looked up in, and the name), and a walk of several recursive probes along the same column (a second
//! step that climbs on where another test holds), have no reach per node: a node's successors depend on the row. They
//! run as a worklist instead: the seeds, then each probe's rest for every row made at the last depth, with all of the
//! row's slots bound, until a depth makes no new row. That is the climb W's lookup does for each reference, read from
//! the rule. It needs no rounds of the component (a wave, its runs and lazy indexes, its delta): one wave in all.
//!
//! The form runs in two halves around its launches (`prepare`, `finish`), so a stratum can wait for them while the
//! other strata of its level prepare theirs: the launches of a level then go to the executor as one batch (a device
//! wave, decision 0095 §5, `lanes::wave`).

use rerail_blocks::vector::Vector;
use super::{Body, Component, JoinRule, Key, SEED, Src, Step, Store, Term, run_join_each, run_join_each_row};
use super::steps::{Src as MadeSrc, Step as Made};
use rerail_section::Decode;
use std::sync::Arc;
use crate::code_rule_dsl::logic::lanes::prof;
use rerail_accel::{Bits, Kernel, Output};
use rerail_blocks::maps::BlockHash;

/// The most nodes of one launch's bit rows: 16,384² bits are 32 MB.
pub const MAX_DENSE: usize = 16_384;
/// Components are packed into pieces of about this many nodes (a larger component is a piece alone).
const PACK: usize = 1_024;
/// The most bits a closure's launches hold at once (128 MB): past it, the largest pieces are walked on the host.
const MAX_BITS: u64 = 1 << 30;

/// A recognized closure: the walking column, the probe's rest as a join from `y` to `z`, and the slot of `y`. Keyed
/// (decision 0570): each recursive probe's rest instead, run per row (`keyed`); `rest` and `y` are then unused.
pub(super) struct Closure {
    col: usize,
    rest: JoinRule,
    y: usize,
    keyed: Vec<Keyed>,
    /// The walking relation's index in the component (0 unless it has readers).
    w: usize,
    /// The climb's ends (decision 0561): the component's other relations, each made from the walk's rows one at a
    /// time, run in the keyed climb at each depth on the rows it made.
    readers: Vec<Reader>,
    /// The row tests the rests share (decision 0561): an atom keyed by the row's columns alone (`bound_in(s, x)`,
    /// `cls_late(n, x, s)`), as a join of that one atom over the row's columns (slot `c`: column `c`), asked once a
    /// row however many rests read it, with whichever sign.
    /// Each is asked as the index's probe alone by its one step (`cands::holds`).
    tests: Vec<JoinRule>,
}

/// Whether two row tests ask the same: the same source by the same index, the same keys.
fn same_test(a: &Made, b: &Made) -> bool {
    let (Made::Atom { src: MadeSrc::In(ra, ia), keys: ka, .. }, Made::Atom { src: MadeSrc::In(rb, ib), keys: kb, .. }) = (a, b) else { return false };
    Arc::ptr_eq(&ra.0, &rb.0) && ia.0.cols == ib.0.cols && ka.len() == kb.len() && ka.iter().zip(kb).all(|(x, y)| x.0 == y.0 && match (x.1, y.1) {
        (Key::Slot(p), Key::Slot(q)) => p == q,
        (Key::Const(p), Key::Const(q)) => p == q,
        _ => false,
    })
}

/// A rest's row tests taken out (decision 0561): each atom of an input that binds nothing and is keyed only by the
/// row's slots (`slots`, by column) or constants, as a test over the row's columns, shared through `tests`. The rest
/// left, and each test it needs with its sign.
fn take_tests(j: &JoinRule, rest: &[Step], slots: &[usize], tests: &mut Vec<Made>) -> (Vec<Made>, Vec<(usize, bool)>) {
    let names = j.names();
    let (mut left, mut need) = (Vec::new(), Vec::new());
    for st in rest {
        let test = match st {
            Step::Atom { src: src @ Src::In(..), neg, keys, binds, tests: eqs } if binds.is_empty() && eqs.is_empty() => keys
                .iter()
                .map(|&(c, key)| match key {
                    Key::Slot(x) => slots.iter().position(|&y| y == x).map(|col| (c, Key::Slot(col))),
                    Key::Const(v) => Some((c, Key::Const(v))),
                })
                .collect::<Option<Vec<_>>>()
                .map(|keys| (Made::Atom { src: <MadeSrc as Decode>::decode(src, &names), neg: false, keys, binds: Vec::new(), tests: Vec::new() }, *neg)),
            _ => None,
        };
        match test {
            Some((t, neg)) => {
                let at = match tests.iter().position(|u| same_test(u, &t)) {
                    Some(at) => at,
                    None => {
                        tests.push(t);
                        tests.len() - 1
                    }
                };
                need.push((at, !neg));
            }
            None => left.push(<Made as Decode>::decode(st, &names)),
        }
    }
    (left, need)
}

/// A reader of the walk's rows (decision 0561): its rest as a join to its head's terms, run with every column's slot
/// (`slots`, by column) bound to a walk row's values, its rows into relation `head`.
pub(super) struct Reader {
    head: usize,
    rest: JoinRule,
    slots: Vec<usize>,
    need: Vec<(usize, bool)>,
    /// Its head's terms, as columns of the row or constants, when its rest is all row tests: then nothing is run for
    /// it, and a row that passes them is its row.
    direct: Option<Vec<Result<usize, i64>>>,
}

/// One recursive probe of a keyed closure: its rest as a join to the new value `z`, run with every column's slot
/// (`slots`, by column) bound to a row's values.
pub(super) struct Keyed {
    rest: JoinRule,
    slots: Vec<usize>,
    /// The row tests (decision 0561) its rest had, taken out of it: each a shared test and whether it must hold.
    need: Vec<(usize, bool)>,
}

/// A recursive probe read as a walk: the walking column, each column's slot, the new value's slot, and whether the
/// rest reads another column's slot (it is then keyed).
struct Walk<'a> {
    j: &'a JoinRule,
    col: usize,
    slot_of: Vec<usize>,
    z: usize,
    rest: &'a [Step],
    keyed: bool,
}

/// Whether probe `j` of relation 0 (of `k` columns) is a walk: its delta atom first, a distinct slot in every column,
/// the head the same slots but one column's, which is a slot the rest binds; the rest reads the relation nowhere.
fn walk_of(j: &JoinRule, k: usize, w: usize, nrels: usize) -> Option<Walk<'_>> {
    let Some(Step::Atom { src: Src::Delta(d), neg: false, keys, binds, tests }) = j.steps.first() else { return None };
    if *d != w {
        return None;
    }
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
    let others: Vec<usize> = (0..k).filter(|&c| c != col).map(|c| slot_of[c]).collect();
    let rest = &j.steps[1..];
    let mut used = Vector::new();
    for s in rest {
        if (0..nrels).any(|r| reads(s, r)) {
            return None;
        }
        step_slots(s, &mut used);
    }
    if !used.contains(&z) {
        return None;
    }
    let keyed = used.iter().any(|s| others.contains(s));
    Some(Walk { j, col, slot_of, z, rest, keyed })
}

/// A reader's probe (decision 0561): its walk atom first, reading the walk's delta with a distinct slot in every
/// column; its rest reads none of the component's relations. Each column's slot, and the rest.
fn reader_of(j: &JoinRule, k: usize, w: usize, nrels: usize) -> Option<(Vec<usize>, &[Step])> {
    let Some(Step::Atom { src: Src::Delta(d), neg: false, keys, binds, tests }) = j.steps.first() else { return None };
    if *d != w || !keys.is_empty() || !tests.is_empty() || binds.len() != k {
        return None;
    }
    let mut slot_of = vec![usize::MAX; k];
    for &(c, s) in binds {
        slot_of[c] = s;
    }
    if slot_of.contains(&usize::MAX) || (1..k).any(|i| slot_of[..i].contains(&slot_of[i])) {
        return None;
    }
    let rest = &j.steps[1..];
    if rest.iter().any(|s| (0..nrels).any(|r| reads(s, r))) {
        return None;
    }
    Some((slot_of, rest))
}

/// A rest's steps made again, its run's objects the same ones (`steps::Plan::decoded`'s), to be a join of their own.
fn decoded(j: &JoinRule, rest: &[Step]) -> Vec<Made> {
    let names = j.names();
    rest.iter().map(|s| <Made as Decode>::decode(s, &names)).collect()
}

/// A rest as its own join, to the new value `z`.
fn rest_join(rest: Vec<Made>, z: usize, nslots: usize, bound: &[usize]) -> JoinRule {
    rest_join_to(rest, vec![Term::Slot(z)], nslots, bound)
}

/// A rest as its own join, to the terms `out`, `bound` the slots its row binds before it (the removed first step's:
/// a range's ends may be among them, `super::ranges_from`). Its probes and ranges are read off its steps' view, once
/// encoded, and it is encoded again with them.
fn rest_join_to(rest: Vec<Made>, out: Vec<Term>, nslots: usize, bound: &[usize]) -> JoinRule {
    let n = rest.len();
    let mut plan = super::steps::Plan {
        steps: rest,
        out,
        nslots,
        multi: Vec::new(),
        probes: Vec::new(),
        fused: (0..n).map(|_| None).collect(),
        spec: 0,
        from: (0..n).collect(),
        plan_steps: n,
        ranges: Vec::new(),
    };
    let drafted = plan.encode();
    plan.probes = super::probes(&drafted.steps, &drafted.out);
    plan.ranges = super::ranges_from(&drafted.steps, bound);
    JoinRule::encoded(plan.encode())
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
        Step::Split { whole, a, b, ka, kb, .. } => {
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
    if !c.edges.is_empty() || !c.gates.is_empty() || !c.requests.is_empty() || !c.produced.is_empty() {
        return None;
    }
    let nrels = store.rels.len();
    // A lattice relation folds each row it is given, one a key; the closure writes rows by distinct node and pends its
    // seeds as a plain relation's. A component holding one runs semi-naive.
    if store.rels.iter().any(|r| r.lat.is_some()) {
        return None;
    }
    // The walking relation: the one a probe reads the delta of to make more of (with readers beside it, decision 0561).
    let wi = c.probes.iter().find(|p| matches!(p.body, Body::Join(_)) && p.delta == p.head && p.delta != SEED).map(|p| p.head)?;
    let ks: Vec<usize> = store.rels.iter().map(|r| r.k).collect();
    drop(store);
    let k = ks[wi];
    let (mut walks, mut readers) = (Vec::new(), Vec::new());
    for p in &c.probes {
        match (&p.body, p.delta) {
            (Body::Join(_), SEED) if p.head == wi => {}
            (Body::Join(j), d) if d == wi && p.head == wi => walks.push((walk_of(j, k, wi, nrels)?, j.nslots)),
            (Body::Join(j), d) if d == wi && j.out.len() == ks[p.head] => {
                let (slots, rest) = reader_of(j, k, wi, nrels)?;
                readers.push((p.head, slots, rest, j));
            }
            _ => return None,
        }
    }
    // Each other relation is a reader's head, and only a reader's.
    if (0..nrels).any(|r| r != wi && !readers.iter().any(|x| x.0 == r)) {
        return None;
    }
    let col = walks.first()?.0.col;
    if walks.iter().any(|(w, _)| w.col != col) {
        return None;
    }
    // One walk whose rest reads only the walking column: the reach per node.
    if let [(w, nslots)] = &walks[..] && !w.keyed && readers.is_empty() && wi == 0 {
        return Some(Closure { col, rest: rest_join(decoded(w.j, w.rest), w.z, *nslots, &w.slot_of), y: w.slot_of[col], keyed: Vec::new(), w: 0, readers: Vec::new(), tests: Vec::new() });
    }
    // Keyed (decision 0570): `RERAIL_KEYED_CLOSURE=off` leaves it semi-naive.
    if std::env::var("RERAIL_KEYED_CLOSURE").is_ok_and(|v| v == "off") {
        return None;
    }
    // The rests' row tests, shared (decision 0561): only where there is more than one rest to share them.
    let share = walks.len() + readers.len() > 1 && std::env::var("RERAIL_ROW_TESTS").map_or(true, |v| v != "off");
    let mut tests: Vec<Made> = Vec::new();
    let mut split = |j: &JoinRule, rest: &[Step], slots: &[usize]| if share { take_tests(j, rest, slots, &mut tests) } else { (decoded(j, rest), Vec::new()) };
    let keyed = walks
        .iter()
        .map(|(w, nslots)| {
            let (left, need) = split(w.j, w.rest, &w.slot_of);
            Keyed { rest: rest_join(left, w.z, *nslots, &w.slot_of), slots: w.slot_of.clone(), need }
        })
        .collect();
    let readers = readers
        .into_iter()
        .map(|(head, slots, rest, j)| {
            let (left, need) = split(j, rest, &slots);
            let direct = left.is_empty().then(|| j.out.iter().map(|t| match *t {
                Term::Slot(x) => slots.iter().position(|&y| y == x).map(Ok),
                Term::Const(v) => Some(Err(v)),
                Term::Cat(..) => None,
            }).collect::<Option<Vec<_>>>()).flatten();
            // A head the row's columns do not make keeps its rest (its tests too): it is run.
            let (left, need) = if left.is_empty() && direct.is_none() { (decoded(j, rest), Vec::new()) } else { (left, need) };
            Reader { head, rest: rest_join_to(left, j.out.to_vec(), j.nslots, &slots), slots, need, direct }
        })
        .collect();
    let all: Vec<usize> = (0..k).collect();
    let tests = tests.into_iter().map(|t| rest_join_to(vec![t], vec![Term::Const(1)], k, &all)).collect();
    let (w, nslots) = &walks[0];
    Some(Closure { col, rest: rest_join(decoded(w.j, w.rest), w.z, *nslots, &w.slot_of), y: w.slot_of[col], keyed, w: wi, readers, tests })
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
    /// A keyed closure's run (decision 0570), done in the first half: the rows it made and its depths.
    keyed: Option<(usize, usize)>,
}

/// The first half: the distinct start nodes, the edges out of every node reached (the rest, searched once per
/// node), then the reach graph's components, and the pieces' launches. `None` when the search fails: the caller then
/// runs semi-naive from the seeds.
pub(super) fn prepare(cl: &Closure, store: &mut Store, syms: &mut super::Interner) -> Option<Prepared> {
    if !cl.keyed.is_empty() {
        return keyed(cl, store, syms);
    }
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
        return Some(Prepared { nodes, index, adj, at: Vector::new(), pieces: Lists::new(), launches: Vec::new(), levels, keyed: None });
    }
    let _h = prof::host("closure", "encode: components, pieces, adjacency");
    let (at, pieces, launches) = partition(&adj);
    Some(Prepared { nodes, index, adj, at, pieces, launches, levels: false, keyed: None })
}

/// The keyed closure (decision 0570): from the seeds, each depth's rows run every probe's rest with the row's slots
/// bound, and the rows they make that are new are the next depth's. A relation that keeps levels (decision 0090) is
/// left to semi-naive (`None`), which gives each row its depth.
fn keyed(cl: &Closure, store: &mut Store, syms: &mut super::Interner) -> Option<Prepared> {
    let _h = prof::host("closure", "keyed: each depth's rows through the steps");
    let wi = cl.w;
    if store.rels.iter().any(|r| r.level.len() == r.n && r.n > 0) {
        return None;
    }
    let k = store.rels[wi].k;
    let mut frontier: Vector<i64> = Vector::new();
    {
        let rel = &store.rels[wi];
        for id in 0..rel.n {
            if !rel.dead(id as u32) {
                frontier.extend_from_slice(&rel.row(id));
            }
        }
    }
    let (mut rows, mut depths) = (0usize, 0usize);
    let mut made: Vector<i64> = Vector::new();
    let all: Vector<usize> = (0..k).collect();
    // Each shared test's answer for each row of the depth: 0 not asked yet, 1 holds, 2 does not.
    let mut answers: Vec<Vector<u8>> = (0..cl.tests.len()).map(|_| Vector::new()).collect();
    let mut ends: Vector<i64> = Vector::new();
    // A test asked as the index's probe alone (the spike, decision 0561); `RERAIL_DIRECT_TESTS=off` asks it as a join.
    let direct = std::env::var("RERAIL_DIRECT_TESTS").map_or(true, |v| v != "off");
    let asked: Vec<std::cell::RefCell<super::cands::Asked>> = cl.tests.iter().map(|_| Default::default()).collect();
    while !frontier.is_empty() {
        depths += 1;
        made.clear();
        let n = frontier.len() / k;
        for a in &mut answers {
            a.clear();
            a.resize(n, 0);
        }
        // The rows that pass a rest's tests, each test asked of the rows not yet answered (decision 0561).
        let passing = |need: &[(usize, bool)], answers: &mut Vec<Vector<u8>>, store: &Store, syms: &mut super::Interner| -> Option<Vector<u32>> {
            let mut alive: Vector<u32> = (0..n as u32).collect();
            for &(t, want) in need {
                let ask: Vector<u32> = alive.iter().copied().filter(|&i| answers[t][i as usize] == 0).collect();
                if !ask.is_empty() && direct {
                    let mut a = asked[t].borrow_mut();
                    for &r in ask.iter() {
                        let r = r as usize;
                        let row = &frontier[r * k..(r + 1) * k];
                        answers[t][r] = if super::cands::holds(&cl.tests[t], &cl.tests[t].steps[0], store, row, &mut a) { 1 } else { 2 };
                    }
                } else if !ask.is_empty() {
                    let mut i = 0;
                    let got = &mut answers[t];
                    run_join_each_row(&cl.tests[t], store, syms, &all, &mut |out: &mut Vector<i64>, vals: &mut [i64]| {
                        if i > 0 {
                            got[ask[i - 1] as usize] = if out.is_empty() { 2 } else { 1 };
                        }
                        if i == ask.len() {
                            return false;
                        }
                        let r = ask[i] as usize;
                        vals.copy_from_slice(&frontier[r * k..(r + 1) * k]);
                        i += 1;
                        true
                    })
                    .ok()?;
                }
                let a = &answers[t];
                alive.retain(|&i| (a[i as usize] == 1) == want);
            }
            Some(alive)
        };
        // The climb's ends: each reader over the rows this depth starts from (the seeds, then each depth's new rows).
        let ends_h = (!cl.readers.is_empty()).then(|| prof::host("closure", "keyed: the readers (the climb's ends, decision 0561)"));
        for rd in &cl.readers {
            let tests_h = prof::host("closure", "keyed: the readers' tests");
            let alive = passing(&rd.need, &mut answers, store, syms)?;
            drop(tests_h);
            ends.clear();
            match &rd.direct {
                Some(terms) => {
                    for &i in alive.iter() {
                        let row = &frontier[i as usize * k..(i as usize + 1) * k];
                        ends.extend(terms.iter().map(|t| match *t { Ok(c) => row[c], Err(v) => v }));
                    }
                }
                None => {
                    let mut i = 0;
                    let got = &mut ends;
                    run_join_each_row(&rd.rest, store, syms, &rd.slots, &mut |out: &mut Vector<i64>, vals: &mut [i64]| {
                        if i > 0 {
                            got.extend_from_slice(out);
                        }
                        if i == alive.len() {
                            return false;
                        }
                        let r = alive[i] as usize;
                        vals.copy_from_slice(&frontier[r * k..(r + 1) * k]);
                        i += 1;
                        true
                    })
                    .ok()?;
                }
            }
            let _push_h = prof::host("closure", "keyed: the readers' rows pushed");
            let rel = &mut store.rels[rd.head];
            let hk = rel.k;
            for row in ends.chunks(hk) {
                rows += rel.push(row) as usize;
            }
        }
        drop(ends_h);
        for kp in &cl.keyed {
            let alive = passing(&kp.need, &mut answers, store, syms)?;
            let mut i = 0;
            run_join_each_row(&kp.rest, store, syms, &kp.slots, &mut |out: &mut Vector<i64>, vals: &mut [i64]| {
                if i > 0 {
                    let r = alive[i - 1] as usize;
                    let row = &frontier[r * k..(r + 1) * k];
                    for &z in out.iter() {
                        let at = made.len();
                        made.extend_from_slice(row);
                        made[at + cl.col] = z;
                    }
                }
                if i == alive.len() {
                    return false;
                }
                let r = alive[i] as usize;
                vals.copy_from_slice(&frontier[r * k..(r + 1) * k]);
                i += 1;
                true
            })
            .ok()?;
        }
        frontier.clear();
        let rel = &mut store.rels[wi];
        for row in made.chunks(k) {
            if rel.push(row) {
                frontier.extend_from_slice(row);
                rows += 1;
            }
        }
    }
    prof::count("closure", "keyed closures", 1);
    prof::count("closure", "keyed closure depths", depths as u64);
    Some(Prepared { nodes: Vector::new(), index: BlockHash::default(), adj: Lists::new(), at: Vector::new(), pieces: Lists::new(), launches: Vec::new(), levels: false, keyed: Some((rows, depths)) })
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
    if let Some((rows, depths)) = p.keyed {
        return Some(Ran { nodes: 0, rows, how: format!("keyed steps over {depths} depths") });
    }
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
