//! X4, design (d) (docs/facts/reports/explore-x4-facts.md): maintain only the declared,
//! membership-dependent facts the repository plans read, recompute every clause's own stages per
//! edit.
//!
//! - `frontier`: the nodes of a normalised plan that are declared facts rather than rule logic.
//!   The facts are the plan's reference graph (`reference_edges` and the linker views it is built
//!   from: every node first built for a `views/*` stage, `module_of` among them); the frontier is
//!   the set of fact nodes some rule node (or an output) reads. Everything a rule node reads past
//!   the frontier is the maintained table; everything else is recomputed.
//! - `scope`: the demand rewrite's side conditions, checked per output on the graph (no names,
//!   I3). An output is scoped by key column `i` when its rows with key `k` are a function of the
//!   leaf rows with key `k` only:
//!   S1 `Map` keeps the key as a bare column; `Filter`, `Distinct`, `Affinity` keep it;
//!   S2 `Join`/`Semi` (anti included): the key side's key column is equated by the join's keys
//!      with a key column of the other side, or the other side reads no leaf at all;
//!   S3 `Concat`: every input keyed (or leaf-free);
//!   S4 `Group`: the key is a grouping column (`keyless` fails); `Window`: the key is a partition
//!      column; `DistinctPrefix`: the key is inside the prefix;
//!   S5 `Fix`: init and step keep the key with the variable assumed keyed at the same column (a
//!      recursion that moves the key, e.g. a join of the variable on another column, fails);
//!   S6 leaves (`Exported`, frontier nodes): the key column of the leaf. The edit's keys are the
//!      key values of the changed blocks' rows (old and new contents), every leaf is restricted to
//!      rows with those keys across all members, and the output's rows with those keys replace
//!      the stored ones.
//!   A plan is scoped when every output is; otherwise it is recomputed from scratch.
use super::lower;
use super::norm::{Graph, KeyMode, NodeId, Op, X};
use rerail_facts::ir::Expr;
use rustc_hash::FxHashMap;
use serde_json::json;

fn is_fact(label: &str) -> bool {
    label.contains("/views/")
}

/// Nodes needed by `g`'s outputs.
fn needed(g: &Graph) -> Vec<bool> {
    let mut need = vec![false; g.nodes.len()];
    let mut stack: Vec<NodeId> = g.outputs.iter().map(|(_, o, _)| *o).collect();
    while let Some(x) = stack.pop() {
        if !need[x] {
            need[x] = true;
            stack.extend(lower::op_inputs(&g.nodes[x]));
        }
    }
    need
}

/// The declared-fact nodes a rule node or an output reads (see the module doc).
pub fn frontier(g: &Graph) -> Vec<NodeId> {
    let need = needed(g);
    let mut f = vec![false; g.nodes.len()];
    for x in (0..g.nodes.len()).filter(|x| need[*x] && !is_fact(&g.labels[*x])) {
        for i in lower::op_inputs(&g.nodes[x]) {
            if is_fact(&g.labels[i]) {
                f[i] = true;
            }
        }
    }
    for (_, o, _) in &g.outputs {
        if is_fact(&g.labels[*o]) {
            f[*o] = true;
        }
    }
    (0..g.nodes.len()).filter(|x| f[*x]).collect()
}

/// Nodes the frontier is computed from (the maintained part), and those recomputed.
pub fn split(g: &Graph) -> (Vec<bool>, Vec<bool>) {
    let need = needed(g);
    let fr = frontier(g);
    let mut maint = vec![false; g.nodes.len()];
    let mut stack = fr.clone();
    while let Some(x) = stack.pop() {
        if !maint[x] {
            maint[x] = true;
            stack.extend(lower::op_inputs(&g.nodes[x]));
        }
    }
    // Recomputed: needed nodes reachable from the outputs without passing through the frontier.
    let mut rule = vec![false; g.nodes.len()];
    let is_fr: Vec<bool> = (0..g.nodes.len()).map(|x| fr.contains(&x)).collect();
    let mut stack: Vec<NodeId> = g.outputs.iter().map(|(_, o, _)| *o).filter(|o| !is_fr[*o]).collect();
    while let Some(x) = stack.pop() {
        if !rule[x] {
            rule[x] = true;
            stack.extend(lower::op_inputs(&g.nodes[x]).into_iter().filter(|i| !is_fr[*i]));
        }
    }
    let _ = need;
    (maint, rule)
}

fn bare(e: &X) -> Option<usize> {
    match e {
        Expr::Col { depth: 0, col } => Some(*col as usize),
        _ => None,
    }
}

struct Scope<'a> {
    g: &'a Graph,
    leaf: Vec<bool>,
    free: Vec<Option<bool>>,
    memo: FxHashMap<(NodeId, usize), Result<(), String>>,
    vars: FxHashMap<u32, usize>,
    /// Leaf key columns used: (node, column).
    leaves: Vec<(NodeId, usize)>,
}

impl Scope<'_> {
    /// Does `n` read no leaf (only constants)?
    fn leaf_free(&mut self, n: NodeId) -> bool {
        if let Some(b) = self.free[n] {
            return b;
        }
        let b = !self.leaf[n] && !matches!(self.g.nodes[n], Op::Exported(_) | Op::Var(_)) && lower::op_inputs(&self.g.nodes[n]).into_iter().all(|i| self.leaf_free(i));
        self.free[n] = Some(b);
        b
    }

    fn fail(&self, n: NodeId, why: &str) -> Result<(), String> {
        Err(format!("{} (n{n}): {why}", self.g.labels[n]))
    }

    /// The other side of a join or semijoin, keyed on the column the key side's `col` is equated
    /// with, or leaf-free.
    fn other(&mut self, n: NodeId, col: usize, mine: &[(X, KeyMode)], theirs: &[(X, KeyMode)], side: NodeId) -> Result<(), String> {
        if self.leaf_free(side) {
            return Ok(());
        }
        for (a, b) in mine.iter().zip(theirs) {
            if bare(&a.0) == Some(col) {
                if let Some(j) = bare(&b.0) {
                    return self.trace(side, j);
                }
            }
        }
        self.fail(n, "the other side is not equated on the key (S2)")
    }

    fn trace(&mut self, n: NodeId, col: usize) -> Result<(), String> {
        if self.leaf[n] {
            self.leaves.push((n, col));
            return Ok(());
        }
        if let Some(r) = self.memo.get(&(n, col)) {
            return r.clone();
        }
        let g = self.g;
        let r = match &g.nodes[n] {
            Op::Exported(_) => {
                self.leaves.push((n, col));
                Ok(())
            }
            Op::Const(_) => Ok(()),
            Op::Var(v) => match self.vars.get(v) {
                Some(c) if *c == col => Ok(()),
                _ => self.fail(n, "the recursion moves the key (S5)"),
            },
            Op::Map { input, exprs } => match exprs.get(col).and_then(bare) {
                Some(j) => self.trace(*input, j),
                None => self.fail(n, "the key is computed, not a bare column (S1)"),
            },
            Op::Filter { input, .. } | Op::Distinct { input } | Op::Affinity { input, .. } => self.trace(*input, col),
            Op::Join { l, r, lk, rk } => {
                let lw = g.widths[*l];
                if col < lw {
                    self.trace(*l, col).and_then(|_| self.other(n, col, lk, rk, *r))
                } else {
                    self.trace(*r, col - lw).and_then(|_| self.other(n, col - lw, rk, lk, *l))
                }
            }
            Op::Semi { l, r, lk, rk, .. } => self.trace(*l, col).and_then(|_| self.other(n, col, lk, rk, *r)),
            Op::Concat { inputs } => {
                let mut r = Ok(());
                for i in inputs.clone() {
                    if !self.leaf_free(i) {
                        r = r.and_then(|_| self.trace(i, col));
                    }
                }
                r
            }
            Op::Group { input, keys, keyless, .. } => {
                if *keyless || !keys.iter().any(|(k, _)| bare(k) == Some(col)) {
                    self.fail(n, "aggregate over rows of several keys (S4)")
                } else {
                    self.trace(*input, col)
                }
            }
            Op::Window { input, part, .. } => {
                if part.iter().any(|(k, _)| bare(k) == Some(col)) {
                    self.trace(*input, col)
                } else {
                    self.fail(n, "window partition does not contain the key (S4)")
                }
            }
            Op::DistinctPrefix { input, prefix } => {
                if col < *prefix {
                    self.trace(*input, col)
                } else {
                    self.fail(n, "DISTINCT prefix does not contain the key (S4)")
                }
            }
            Op::Fix { var, init, step } => {
                let prev = self.vars.insert(*var, col);
                let mut r = Ok(());
                for i in init.iter().chain(step) {
                    r = r.and_then(|_| self.trace(*i, col));
                }
                match prev {
                    Some(p) => self.vars.insert(*var, p),
                    None => self.vars.remove(var),
                };
                r
            }
        };
        // Results under a recursion assumption are not memoised (they depend on it).
        if self.vars.is_empty() {
            self.memo.insert((n, col), r.clone());
        }
        r
    }
}

/// Per output: the first key column the side conditions admit (with its leaf key columns), or
/// each column's first failure.
pub fn scope(g: &Graph, maintained: bool) -> serde_json::Value {
    let fr = if maintained { frontier(g) } else { Vec::new() };
    let mut leaf = vec![false; g.nodes.len()];
    for f in &fr {
        leaf[*f] = true;
    }
    let mut outs = Vec::new();
    let mut all = true;
    for (clause, o, cols) in &g.outputs {
        let mut sc = Scope { g, leaf: leaf.clone(), free: vec![None; g.nodes.len()], memo: FxHashMap::default(), vars: FxHashMap::default(), leaves: Vec::new() };
        let mut fails = Vec::new();
        let mut ok = None;
        for c in 0..g.widths[*o] {
            sc.leaves.clear();
            match sc.trace(*o, c) {
                Ok(()) => {
                    let mut lv: Vec<String> = sc.leaves.iter().map(|(n, c)| format!("{} col {c}", g.labels[*n])).collect();
                    lv.sort();
                    lv.dedup();
                    ok = Some(json!({"column": cols.get(c).cloned().unwrap_or_else(|| c.to_string()), "leaves": lv}));
                    break;
                }
                Err(e) => fails.push(json!([cols.get(c).cloned().unwrap_or_else(|| c.to_string()), e])),
            }
        }
        all &= ok.is_some();
        outs.push(json!({"output": clause, "scoped": ok, "failures": if ok.is_some() { json!([]) } else { json!(fails) }}));
    }
    json!({"scoped": all, "outputs": outs})
}

/// `facts-b dgraph --real SPEC`: per plan the declared-fact split (frontier nodes and their
/// stages; nodes maintained vs recomputed) and the demand rewrite's verdict, with and without the
/// frontier as leaves.
pub fn report(spec: &std::path::Path) {
    let st = crate::store::Store::default();
    super::val::install(std::rc::Rc::new(std::cell::RefCell::new(crate::derive::World::new(st))));
    let real = super::real::load(spec);
    let mut plans = Vec::new();
    for (name, g) in &real.graphs {
        let fr = frontier(g);
        let (maint, rule) = split(g);
        let mut stages_rule: Vec<String> = (0..g.nodes.len()).filter(|x| rule[*x]).map(|x| g.labels[x].clone()).collect();
        stages_rule.sort();
        stages_rule.dedup();
        let mut stages_maint: Vec<String> = (0..g.nodes.len()).filter(|x| maint[*x]).map(|x| g.labels[x].clone()).collect();
        stages_maint.sort();
        stages_maint.dedup();
        let both: Vec<String> = (0..g.nodes.len()).filter(|x| maint[*x] && rule[*x]).map(|x| g.labels[x].clone()).collect();
        plans.push(json!({
            "plan": name, "nodes": g.nodes.len(),
            "frontier": fr.iter().map(|x| json!([x, g.labels[*x], g.widths[*x]])).collect::<Vec<_>>(),
            "maintained_nodes": maint.iter().filter(|b| **b).count(), "recomputed_nodes": rule.iter().filter(|b| **b).count(), "shared_nodes": both,
            "maintained_stages": stages_maint, "recomputed_stages": stages_rule,
            "scope_all_recomputed": scope(g, false),
            "scope_frontier_as_leaves": scope(g, true),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"refused": real.refused, "plans": plans})).unwrap());
}

fn json_of(v: &rerail_facts::sql::value::V) -> serde_json::Value {
    use rerail_facts::sql::value::V;
    match v {
        V::Null => serde_json::Value::Null,
        V::Int(i) => json!(i),
        V::Real(f) => json!(f),
        V::Text(s) => json!(&**s),
    }
}

/// `facts-b dref --real SPEC --worktree N --out DIR`: one worktree from scratch on `batch`: per
/// plan its frontier relations (`DIR/<plan>.<label>.jsonl`, one row per line, the maintained
/// table the X4 harness hands to another engine) and its outputs (`DIR/<plan>.out.jsonl`), with
/// the seconds of each part.
pub fn dref(spec: &std::path::Path, worktree: usize, out: &std::path::Path) {
    use crate::store::{self, Cid, Store, Sym};
    use std::io::Write;
    std::fs::create_dir_all(out).unwrap();
    let sd = store::store_dir();
    let ms = crate::base::load_memberships();
    let (name, m) = &ms[worktree - 1];
    let mut st = Store { ids_only: true, ..Default::default() };
    for (_, h) in m {
        st.load(&sd, h);
    }
    let world = std::rc::Rc::new(std::cell::RefCell::new(crate::derive::World::new(st)));
    super::val::install(world.clone());
    let real = std::rc::Rc::new(super::real::load(spec));
    let mut files: Vec<(Sym, Cid)> = Vec::new();
    let mut rows: FxHashMap<(u32, Sym, Cid), std::rc::Rc<[super::val::Row]>> = FxHashMap::default();
    for (p, h) in m {
        let (s, c) = {
            let mut w = world.borrow_mut();
            (w.path(p), w.store.by_hash[h])
        };
        files.push((s, c));
        let hash = crate::facts::hex(h);
        for k in 0..real.programs() {
            match real.program_rows(k, p, &hash) {
                Ok(rs) => {
                    for (e, r) in rs {
                        rows.insert((e, s, c), r.into());
                    }
                }
                Err(e) => eprintln!("{p}: {e}"),
            }
        }
    }
    let rows = std::rc::Rc::new(rows);
    let r2 = rows.clone();
    let src: lower::Source = std::rc::Rc::new(move |e, p, c| r2.get(&(e, p, c)).cloned().unwrap_or_else(|| std::rc::Rc::from(Vec::new())));
    let mut summary = Vec::new();
    for (plan, g) in &real.graphs {
        let fr = frontier(g);
        let t = std::time::Instant::now();
        let (o, pre) = super::batch::eval_with(plan, g, &src, &files, &fr);
        let total = t.elapsed().as_secs_f64();
        // The frontier's rows: re-evaluated alone (eval_with releases them after their readers).
        for f in &fr {
            let rel = super::batch::eval_node(plan, g, &src, &files, *f);
            let label = g.labels[*f].replace('/', "_");
            let mut w = std::io::BufWriter::new(std::fs::File::create(out.join(format!("{plan}.{label}.jsonl"))).unwrap());
            for r in rel.iter() {
                let vs: Vec<serde_json::Value> = super::val::row_to_v(r).iter().map(json_of).collect();
                writeln!(w, "{}", serde_json::Value::Array(vs)).unwrap();
            }
        }
        let mut w = std::io::BufWriter::new(std::fs::File::create(out.join(format!("{plan}.out.jsonl"))).unwrap());
        let mut n = 0u64;
        for (clause, m) in &o.outputs {
            for (r, c) in m {
                let vs: Vec<serde_json::Value> = super::val::row_to_v(r).iter().map(json_of).collect();
                writeln!(w, "{}", json!([clause, vs, c])).unwrap();
                n += *c as u64;
            }
        }
        summary.push(json!({"plan": plan, "worktree": name, "files": files.len(), "frontier_s": pre.secs, "rule_s": total - pre.secs, "output_rows": n, "refusals": o.refusals.len(), "counts": o.counts, "frontier_counts": pre.counts}));
    }
    std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}
