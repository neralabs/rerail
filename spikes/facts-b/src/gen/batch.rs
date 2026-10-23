//! M2's rule executor (PHASE-3 § P3a): a normalised repository plan (`norm::Graph`) evaluated
//! from scratch, set at a time, over one worktree's members. Same operators and the same
//! per-record logic as the generator's lowering (`lower::{map_row, key_of, agg_logic,
//! window_logic, distinct_prefix_logic, affinity}`), evaluated over whole relations instead of
//! tagged differences: a join is a hash join on its keys, a semi/antijoin a probe of the right
//! side's key set, GROUP BY / windows / DISTINCT prefixes group by their keys and run the logic
//! once per group, a set fixpoint is semi-naive over hash indexes. With base records only the
//! generator's dataflow computes exactly these relations (every product is base, presence a
//! threshold), so the two are compared output for output and refusal for refusal
//! (`hybrid::check`).
//!
//! Refusals follow the generator's records: a scalar error in a map or filter outside a recursion,
//! a group whose logic refuses, are `n<node>: <construct>`; a construct the recursion cannot
//! evaluate (an antijoin by the recursive relation, an aggregate over it, a nested recursion) is
//! `<plan>: <stage>: <construct>` and empties that operator, as the lowering does at build time.
//!
//! An intermediate relation is dropped as soon as its last consumer has run.
use super::lower::{self, Source, R};
use super::norm::{Graph, Key, KeyMode, NodeId, Op};
use super::val::{self, Row};
use crate::store::{Cid, Sym};
use rerail_facts::sql::value as sv;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::rc::Rc;

type Bag = Rc<Vec<Row>>;

/// Work counts of one evaluation. Algebraic: exported rows read, rows out of every operator,
/// fixpoint rounds and derivations (candidate rows before the round's distinct). Physical: rows
/// inserted into hash indexes and key probes.
#[derive(Default, Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Counts {
    pub rows_read: u64,
    pub rows_out: u64,
    pub fix_rounds: u64,
    pub derivations: u64,
    pub index_rows: u64,
    pub probes: u64,
    /// Largest number of rows held by live intermediate relations at once.
    pub peak_rows: u64,
}

impl Counts {
    pub fn add(&mut self, o: &Counts) {
        self.rows_read += o.rows_read;
        self.rows_out += o.rows_out;
        self.fix_rounds += o.fix_rounds;
        self.derivations += o.derivations;
        self.index_rows += o.index_rows;
        self.probes += o.probes;
        self.peak_rows = self.peak_rows.max(o.peak_rows);
    }
}

/// One plan's result over one worktree: per output its rows (a multiset), and the plan's
/// refusals (distinct refusal rows; findings are withheld while any is live).
pub struct Outcome {
    pub outputs: Vec<(String, BTreeMap<Row, R>)>,
    pub refusals: Vec<Row>,
    pub counts: Counts,
    /// Per stage (`clause/stage` of the node's label): exclusive seconds and rows out.
    pub stages: FxHashMap<String, (f64, u64)>,
}

fn key(row: &Row, k: &Key) -> Option<Row> {
    lower::key_of(row, k, &mut None)
}

struct Ev<'a> {
    name: &'a str,
    g: &'a Graph,
    src: &'a Source,
    files: &'a [(Sym, Cid)],
    done: Vec<Option<Bag>>,
    /// Consumers of each node not yet run (outputs count as one each, never released).
    uses: Vec<u32>,
    refused: FxHashSet<Row>,
    counts: Counts,
    live_rows: u64,
    /// Seconds spent in the current node's inputs (exclusive time per stage).
    child_secs: f64,
    stages: FxHashMap<String, (f64, u64)>,
}

/// Evaluates `g`'s outputs over `files` (one worktree's members).
pub fn eval(name: &str, g: &Graph, src: &Source, files: &[(Sym, Cid)]) -> Outcome {
    let n = g.nodes.len();
    let mut needed = vec![false; n];
    let mut stack: Vec<NodeId> = g.outputs.iter().map(|(_, o, _)| *o).collect();
    while let Some(x) = stack.pop() {
        if needed[x] {
            continue;
        }
        needed[x] = true;
        stack.extend(lower::op_inputs(&g.nodes[x]));
    }
    let mut uses = vec![0u32; n];
    for x in (0..n).filter(|x| needed[*x]) {
        for i in lower::op_inputs(&g.nodes[x]) {
            uses[i] += 1;
        }
    }
    for (_, o, _) in &g.outputs {
        uses[*o] += 1;
    }
    let mut ev = Ev { name, g, src, files, done: vec![None; n], uses, refused: FxHashSet::default(), counts: Counts::default(), live_rows: 0, child_secs: 0.0, stages: FxHashMap::default() };
    let mut outputs = Vec::new();
    for (clause, o, _) in &g.outputs {
        let b = ev.node(*o);
        let mut m: BTreeMap<Row, R> = BTreeMap::new();
        for r in b.iter() {
            *m.entry(r.clone()).or_default() += 1;
        }
        outputs.push((format!("{name}:{clause}"), m));
    }
    let mut refusals: Vec<Row> = ev.refused.into_iter().collect();
    refusals.sort();
    Outcome { outputs, refusals, counts: ev.counts, stages: ev.stages }
}

impl Ev<'_> {
    fn node(&mut self, n: NodeId) -> Bag {
        if let Some(b) = &self.done[n] {
            return b.clone();
        }
        let outer = std::mem::replace(&mut self.child_secs, 0.0);
        let t = std::time::Instant::now();
        let b: Bag = Rc::new(self.compute(n));
        let total = t.elapsed().as_secs_f64();
        let e = self.stages.entry(self.g.labels[n].clone()).or_default();
        e.0 += total - self.child_secs;
        e.1 += b.len() as u64;
        self.child_secs = outer + total;
        self.counts.rows_out += b.len() as u64;
        self.live_rows += b.len() as u64;
        self.counts.peak_rows = self.counts.peak_rows.max(self.live_rows);
        self.done[n] = Some(b.clone());
        for i in lower::op_inputs(&self.g.nodes[n]) {
            self.release(i);
        }
        b
    }

    fn release(&mut self, i: NodeId) {
        if self.uses[i] == 0 {
            return;
        }
        self.uses[i] -= 1;
        if self.uses[i] == 0 {
            if let Some(b) = self.done[i].take() {
                self.live_rows -= b.len() as u64;
            }
        }
    }

    /// Rows of node `n`'s input grouped by `k` (rows whose key is `None` dropped), the logic run
    /// once per group over its members (distinct rows with their counts, in row order), as the
    /// generator's group recompute runs it over the base view. `group`: GROUP BY's arrangement
    /// (an empty row keys the empty group; `sentinel`: the keyless group exists over no rows).
    #[allow(clippy::too_many_arguments)]
    fn grouped(&mut self, n: NodeId, rows: &[Row], k: &Key, logic: Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>>, group: bool, sentinel: bool) -> Vec<Row> {
        let empty = Row::default();
        let mut groups: FxHashMap<Row, BTreeMap<&Row, R>> = FxHashMap::default();
        for r in rows {
            let kk = if group && r.is_empty() { Some(Row::default()) } else { key(r, k) };
            if let Some(kk) = kk {
                *groups.entry(kk).or_default().entry(r).or_default() += 1;
            }
        }
        if sentinel {
            groups.entry(Row::default()).or_default().entry(&empty).or_default();
        }
        let at = format!("n{n}");
        let mut out = Vec::new();
        for (kk, m) in groups {
            let members: Vec<(&Row, R)> = m.into_iter().filter(|(r, c)| *c != 0 && !(sentinel && r.is_empty())).collect();
            if members.is_empty() && !sentinel {
                continue;
            }
            let (rows, refused) = lower::captured(|| logic(&kk, &members));
            if !refused.is_empty() {
                for w in refused {
                    self.refused.insert(lower::refusal_row(&at, &w));
                }
                continue;
            }
            for (r, c) in rows {
                for _ in 0..c {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    fn compute(&mut self, n: NodeId) -> Vec<Row> {
        let g = self.g;
        match &g.nodes[n] {
            Op::Exported(id) => {
                let mut v = Vec::new();
                for (p, c) in self.files {
                    let rows = (self.src)(*id, *p, *c);
                    self.counts.rows_read += rows.len() as u64;
                    v.extend(rows.iter().cloned());
                }
                v
            }
            Op::Const(rows) => rows.clone(),
            Op::Var(_) => panic!("fixpoint variable outside its fixpoint"),
            Op::Map { input, exprs } => {
                let inp = self.node(*input);
                let at = format!("n{n}");
                let mut out = Vec::with_capacity(exprs.len());
                let mut res = Vec::with_capacity(inp.len());
                for row in inp.iter() {
                    let (ok, refused) = lower::captured(|| lower::map_row(exprs, row, &mut out));
                    match refused.first() {
                        Some(r) => {
                            self.refused.insert(lower::refusal_row(&at, r));
                        }
                        None if ok => res.push(Row::new(&out)),
                        None => {}
                    }
                }
                res
            }
            Op::Filter { input, pred } => {
                let inp = self.node(*input);
                let at = format!("n{n}");
                let mut res = Vec::new();
                for row in inp.iter() {
                    let (keep, refused) = lower::captured(|| lower::ev(pred, &val::row_to_v(row)).and_then(|v| sv::truth(&v)) == Some(true));
                    match refused.first() {
                        Some(r) => {
                            self.refused.insert(lower::refusal_row(&at, r));
                        }
                        None if keep => res.push(row.clone()),
                        None => {}
                    }
                }
                res
            }
            Op::Affinity { input, affs } => self.node(*input).iter().map(|r| lower::affinity(r, affs)).collect(),
            Op::Join { l, r, lk, rk } => {
                let lb = self.node(*l);
                let rb = self.node(*r);
                let mut idx: FxHashMap<Row, Vec<u32>> = FxHashMap::default();
                for (i, row) in rb.iter().enumerate() {
                    if let Some(k) = key(row, rk) {
                        idx.entry(k).or_default().push(i as u32);
                    }
                }
                self.counts.index_rows += rb.len() as u64;
                self.counts.probes += lb.len() as u64;
                let mut out = Vec::new();
                for a in lb.iter() {
                    if let Some(hit) = key(a, lk).and_then(|k| idx.get(&k)) {
                        for &i in hit {
                            out.push(Row::concat(a, &rb[i as usize]));
                        }
                    }
                }
                out
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let lb = self.node(*l);
                let rb = self.node(*r);
                let keys: FxHashSet<Row> = rb.iter().filter_map(|row| key(row, rk)).collect();
                self.counts.index_rows += rb.len() as u64;
                self.counts.probes += lb.len() as u64;
                lb.iter().filter(|a| key(a, lk).is_some_and(|k| keys.contains(&k)) != *anti).cloned().collect()
            }
            Op::Distinct { input } => {
                let inp = self.node(*input);
                let mut seen: FxHashSet<&Row> = FxHashSet::default();
                inp.iter().filter(|r| seen.insert(*r)).cloned().collect()
            }
            Op::Concat { inputs } => {
                let mut out = Vec::new();
                for i in inputs.clone() {
                    out.extend(self.node(i).iter().cloned());
                }
                out
            }
            Op::Group { input, keys, aggs, keyless, width, order } => {
                let inp = self.node(*input);
                let logic = lower::agg_logic(keys.clone(), aggs.clone(), *width, order.clone());
                self.grouped(n, &inp, keys, logic, true, *keyless)
            }
            Op::Window { input, part, order, kind } => {
                let inp = self.node(*input);
                self.grouped(n, &inp, part, lower::window_logic(order.clone(), kind.clone()), false, false)
            }
            Op::DistinctPrefix { input, prefix } => {
                let inp = self.node(*input);
                let k: Key = (0..*prefix).map(|c| (super::norm::col(c), KeyMode::Group)).collect();
                self.grouped(n, &inp, &k, lower::distinct_prefix_logic(*prefix), false, false)
            }
            Op::Fix { var, init, step } => self.fix(*var, init, step),
        }
    }

    /// Set fixpoint, semi-naive: every node of the body (reached from `step` through readers of
    /// the variable) and every relation it reads from outside holds its rows so far (`all`, a
    /// set) and the rows new in this round (`delta`). Outside relations are new in round 0 only.
    /// A body operator's new rows are the derivations with at least one new input row (bilinear:
    /// ΔL⋈R_new ∪ L_old⋈ΔR; a semijoin also through the right side's new keys) minus its rows so
    /// far. The variable's new rows are the steps' new rows not yet in it; the result is the
    /// variable's rows when a round adds none. Every body operator is monotone (the lowering's
    /// refusals exclude the others), so this is the least fixpoint the generator's `iterate`
    /// reaches.
    fn fix(&mut self, var: u32, init: &[NodeId], step: &[NodeId]) -> Vec<Row> {
        let g = self.g;
        let rd = lower::var_readers(g, var);
        let mut body = vec![false; g.nodes.len()];
        // Constructs the lowering refuses inside a recursion: the operator is empty (its inputs
        // are not evaluated) and the plan carries a build refusal, as `lower::IterLower` raises it.
        let mut refused_ops: FxHashSet<NodeId> = FxHashSet::default();
        let mut stack: Vec<NodeId> = step.iter().copied().filter(|s| rd[*s]).collect();
        while let Some(x) = stack.pop() {
            if body[x] {
                continue;
            }
            body[x] = true;
            let what = match &g.nodes[x] {
                Op::Semi { r, anti: true, .. } if rd[*r] => Some("antijoin by a recursive relation inside the recursion (not stratified)"),
                Op::Group { .. } | Op::Window { .. } | Op::DistinctPrefix { .. } => Some("aggregate, window or DISTINCT-prefix over a recursive relation (not stratified)"),
                Op::Fix { .. } => Some("nested recursion"),
                _ => None,
            };
            if let Some(w) = what {
                self.refused.insert(lower::refusal_row(self.name, &format!("{}: {w}", g.labels[x])));
                refused_ops.insert(x);
                continue;
            }
            if !matches!(g.nodes[x], Op::Var(_)) {
                stack.extend(lower::op_inputs(&g.nodes[x]).into_iter().filter(|i| rd[*i]));
            }
        }
        let body_nodes: Vec<NodeId> = (0..g.nodes.len()).filter(|x| body[*x]).collect();
        // Outside relations the body reads (a step that does not read the variable is one), and
        // the seeds.
        let mut outer: Vec<NodeId> = step.iter().copied().filter(|s| !rd[*s]).collect();
        for &v in &body_nodes {
            if matches!(g.nodes[v], Op::Var(_)) || refused_ops.contains(&v) {
                continue;
            }
            for i in lower::op_inputs(&g.nodes[v]) {
                if !body[i] && !outer.contains(&i) {
                    outer.push(i);
                }
            }
        }
        let mut st = Semi::default();
        for &o in &outer {
            let b = self.node(o);
            let set: FxHashSet<Row> = b.iter().cloned().collect();
            st.delta.insert(o, set.into_iter().collect());
            st.all.insert(o, FxHashSet::default());
        }
        let mut seeds: FxHashSet<Row> = FxHashSet::default();
        for &i in init {
            seeds.extend(self.node(i).iter().cloned());
        }
        for &v in &body_nodes {
            st.all.insert(v, FxHashSet::default());
        }
        let var_node = body_nodes.iter().copied().find(|v| matches!(g.nodes[*v], Op::Var(x) if x == var));
        let mut var_all: FxHashSet<Row> = FxHashSet::default();
        let mut var_delta: Vec<Row> = seeds.into_iter().collect();
        loop {
            self.counts.fix_rounds += 1;
            for &v in &body_nodes {
                let cand: Vec<Row> = if refused_ops.contains(&v) {
                    Vec::new()
                } else if Some(v) == var_node {
                    var_delta.clone()
                } else {
                    st.step(&g.nodes[v], &mut self.counts)
                };
                self.counts.derivations += cand.len() as u64;
                let all = &st.all[&v];
                let mut fresh: FxHashSet<Row> = FxHashSet::default();
                let d: Vec<Row> = cand.into_iter().filter(|r| !all.contains(r) && fresh.insert(r.clone())).collect();
                st.delta.insert(v, d);
            }
            // The variable's next rows: the steps' new rows not in it yet.
            for r in var_delta.drain(..) {
                var_all.insert(r);
            }
            let mut next: FxHashSet<Row> = FxHashSet::default();
            for &s in step {
                if let Some(d) = st.delta.get(&s) {
                    for r in d {
                        if !var_all.contains(r) {
                            next.insert(r.clone());
                        }
                    }
                }
            }
            st.end_round();
            if next.is_empty() {
                break;
            }
            var_delta = next.into_iter().collect();
        }
        // The body's outside inputs have run their consumers.
        for &v in &body_nodes {
            if matches!(g.nodes[v], Op::Var(_)) {
                continue;
            }
            for i in lower::op_inputs(&g.nodes[v]) {
                if !body[i] {
                    self.release(i);
                }
            }
        }
        var_all.into_iter().collect()
    }
}

/// Semi-naive state: per node its rows so far and this round's new rows; hash indexes of the
/// rows so far by (node, key) and the key sets semijoins probe, both grown at the round's end.
#[derive(Default)]
struct Semi {
    all: FxHashMap<NodeId, FxHashSet<Row>>,
    delta: FxHashMap<NodeId, Vec<Row>>,
    idx: FxHashMap<(NodeId, String), (Key, FxHashMap<Row, Vec<Row>>)>,
    keys: FxHashMap<(NodeId, String), (Key, FxHashSet<Row>)>,
}

impl Semi {
    fn d(&self, n: NodeId) -> &[Row] {
        self.delta.get(&n).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The index of node `n`'s rows so far by `k`, built on first use.
    fn index(&mut self, n: NodeId, k: &Key, c: &mut Counts) -> &FxHashMap<Row, Vec<Row>> {
        let id = (n, format!("{k:?}"));
        if !self.idx.contains_key(&id) {
            let mut m: FxHashMap<Row, Vec<Row>> = FxHashMap::default();
            for r in &self.all[&n] {
                if let Some(kk) = key(r, k) {
                    m.entry(kk).or_default().push(r.clone());
                }
            }
            c.index_rows += self.all[&n].len() as u64;
            self.idx.insert(id.clone(), (k.clone(), m));
        }
        &self.idx[&id].1
    }

    fn keyset(&mut self, n: NodeId, k: &Key, c: &mut Counts) -> &FxHashSet<Row> {
        let id = (n, format!("{k:?}"));
        if !self.keys.contains_key(&id) {
            let s: FxHashSet<Row> = self.all[&n].iter().filter_map(|r| key(r, k)).collect();
            c.index_rows += self.all[&n].len() as u64;
            self.keys.insert(id.clone(), (k.clone(), s));
        }
        &self.keys[&id].1
    }

    /// A body operator's candidate rows this round.
    fn step(&mut self, op: &Op, c: &mut Counts) -> Vec<Row> {
        match op {
            Op::Map { input, exprs } => {
                let mut out = Vec::with_capacity(exprs.len());
                self.d(*input).iter().filter_map(|r| lower::map_row(exprs, r, &mut out).then(|| Row::new(&out))).collect()
            }
            Op::Filter { input, pred } => self.d(*input).iter().filter(|r| lower::ev(pred, &val::row_to_v(r)).and_then(|v| sv::truth(&v)) == Some(true)).cloned().collect(),
            Op::Affinity { input, affs } => self.d(*input).iter().map(|r| lower::affinity(r, affs)).collect(),
            Op::Distinct { input } => self.d(*input).to_vec(),
            Op::Concat { inputs } => inputs.iter().flat_map(|i| self.d(*i).iter().cloned()).collect::<Vec<_>>(),
            Op::Join { l, r, lk, rk } => {
                let (dl, dr) = (self.d(*l).to_vec(), self.d(*r).to_vec());
                let mut out = Vec::new();
                // ΔL ⋈ (R_old ∪ ΔR)
                if !dl.is_empty() {
                    let mut tmp: FxHashMap<Row, Vec<&Row>> = FxHashMap::default();
                    for b in &dr {
                        if let Some(k) = key(b, rk) {
                            tmp.entry(k).or_default().push(b);
                        }
                    }
                    let ro = self.index(*r, rk, c);
                    for a in &dl {
                        c.probes += 1;
                        if let Some(k) = key(a, lk) {
                            for b in ro.get(&k).into_iter().flatten() {
                                out.push(Row::concat(a, b));
                            }
                            for b in tmp.get(&k).into_iter().flatten() {
                                out.push(Row::concat(a, b));
                            }
                        }
                    }
                }
                // L_old ⋈ ΔR
                if !dr.is_empty() {
                    let lo = self.index(*l, lk, c);
                    for b in &dr {
                        c.probes += 1;
                        if let Some(k) = key(b, rk) {
                            for a in lo.get(&k).into_iter().flatten() {
                                out.push(Row::concat(a, b));
                            }
                        }
                    }
                }
                out
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let (dl, dr) = (self.d(*l).to_vec(), self.d(*r).to_vec());
                let dr_keys: FxHashSet<Row> = dr.iter().filter_map(|b| key(b, rk)).collect();
                c.probes += dl.len() as u64;
                if *anti {
                    // The right side is outside the recursion (else refused): complete in round 0.
                    let ks = self.keyset(*r, rk, c);
                    return dl.into_iter().filter(|a| !key(a, lk).is_some_and(|k| ks.contains(&k) || dr_keys.contains(&k))).collect();
                }
                let (mut out, new_keys) = {
                    let old = self.keyset(*r, rk, c);
                    let new_keys: FxHashSet<Row> = dr_keys.into_iter().filter(|k| !old.contains(k)).collect();
                    let out: Vec<Row> = dl.iter().filter(|a| key(a, lk).is_some_and(|k| old.contains(&k) || new_keys.contains(&k))).cloned().collect();
                    (out, new_keys)
                };
                if !new_keys.is_empty() {
                    let lo = self.index(*l, lk, c);
                    for k in &new_keys {
                        out.extend(lo.get(k).into_iter().flatten().cloned());
                    }
                }
                out
            }
            Op::Var(_) | Op::Exported(_) | Op::Const(_) | Op::Group { .. } | Op::Window { .. } | Op::DistinctPrefix { .. } | Op::Fix { .. } => unreachable!("not a body operator"),
        }
    }

    /// New rows join the rows so far, and every index and key set built over them.
    fn end_round(&mut self) {
        for (n, d) in self.delta.drain() {
            if d.is_empty() {
                continue;
            }
            for ((m, _), (k, idx)) in self.idx.iter_mut() {
                if *m == n {
                    for r in &d {
                        if let Some(kk) = key(r, k) {
                            idx.entry(kk).or_default().push(r.clone());
                        }
                    }
                }
            }
            for ((m, _), (k, ks)) in self.keys.iter_mut() {
                if *m == n {
                    ks.extend(d.iter().filter_map(|r| key(r, k)));
                }
            }
            self.all.get_mut(&n).unwrap().extend(d);
        }
    }
}
