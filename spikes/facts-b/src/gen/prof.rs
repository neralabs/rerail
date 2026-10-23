//! Per-node profile of the generated dataflow (`FACTS_PROFILE=1`): every timely operator is
//! attributed to the plan node (and stage) whose lowering built it; per node, scheduled time,
//! records sent on its output channels and records held in its arrangements; per fixpoint and
//! step arm, records per iteration. Written to `<run dir>/profile.json` every 30 s while a
//! load is stepping, and at the end, so a run that never finishes still shows where it is.
use rustc_hash::FxHashMap;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct NodeInfo {
    graph: String,
    node: usize,
    mode: &'static str,
    kind: String,
    label: String,
}

#[derive(Default)]
struct Prof {
    nodes: Vec<NodeInfo>,
    /// operator identifier → index into `nodes`
    op_node: FxHashMap<usize, usize>,
    op_name: FxHashMap<usize, String>,
    op_addr: FxHashMap<usize, Vec<usize>>,
    addr_op: HashMap<Vec<usize>, usize>,
    /// channel → (scope address, source operator index in that scope)
    chan: FxHashMap<usize, (Vec<usize>, usize)>,
    started: FxHashMap<usize, Duration>,
    busy: FxHashMap<usize, Duration>,
    sent_by_chan: FxHashMap<usize, i64>,
    /// (fixpoint label, arm, iteration) → (updates, |Σ diffs|)
    iters: BTreeMap<(String, String, u64), (i64, i64)>,
    notes: BTreeMap<String, serde_json::Value>,
    last: Option<Instant>,
}

thread_local! {
    static ON: bool = std::env::var("FACTS_PROFILE").map_or(false, |v| v != "0");
    static P: RefCell<Prof> = RefCell::new(Prof::default());
}

pub fn on() -> bool {
    ON.with(|o| *o)
}

/// The next operator identifier (call before lowering a node).
pub fn begin(worker: &timely::worker::Worker) -> usize {
    worker.peek_identifier()
}

/// Attributes the operators built since `lo` and not yet attributed to this node (children are
/// lowered first, so each operator goes to the innermost node that built it).
pub fn end(worker: &timely::worker::Worker, lo: usize, graph: &str, node: usize, mode: &'static str, kind: &str, label: &str) {
    let hi = worker.peek_identifier();
    P.with(|p| {
        let mut p = p.borrow_mut();
        let ix = p.nodes.len();
        let mut any = false;
        for id in lo..hi {
            if let std::collections::hash_map::Entry::Vacant(e) = p.op_node.entry(id) {
                e.insert(ix);
                any = true;
            }
        }
        if any {
            p.nodes.push(NodeInfo { graph: graph.to_owned(), node, mode, kind: kind.to_owned(), label: label.to_owned() });
        }
    })
}

/// A named measurement written with the profile.
pub fn note(key: &str, v: serde_json::Value) {
    P.with(|p| {
        p.borrow_mut().notes.insert(key.to_owned(), v);
    })
}

/// Records per iteration of a fixpoint arm (`t.inner`).
pub fn iter(fix: &str, arm: &str, iteration: u64, diff: i64) {
    P.with(|p| {
        let mut p = p.borrow_mut();
        let e = p.iters.entry((fix.to_owned(), arm.to_owned(), iteration)).or_default();
        e.0 += 1;
        e.1 += diff;
    })
}

/// Timely events (from `flow::register_logging`'s timely logger).
pub fn timely_event(time: &Duration, e: &timely::logging::TimelyEvent) {
    use timely::logging::{StartStop, TimelyEvent};
    P.with(|p| {
        let mut p = p.borrow_mut();
        match e {
            TimelyEvent::Operates(o) => {
                p.op_name.insert(o.id, o.name.clone());
                p.op_addr.insert(o.id, o.addr.clone());
                p.addr_op.insert(o.addr.clone(), o.id);
            }
            TimelyEvent::Channels(c) => {
                p.chan.insert(c.id, (c.scope_addr.clone(), c.source.0));
            }
            TimelyEvent::Schedule(s) => match s.start_stop {
                StartStop::Start => {
                    p.started.insert(s.id, *time);
                }
                StartStop::Stop => {
                    if let Some(t0) = p.started.remove(&s.id) {
                        *p.busy.entry(s.id).or_default() += time.saturating_sub(t0);
                    }
                }
            },
            TimelyEvent::Messages(m) if m.is_send => {
                *p.sent_by_chan.entry(m.channel).or_default() += m.record_count;
            }
            _ => {}
        }
    })
}

/// Writes the profile if 30 s passed since the last write (`force`: now).
pub fn tick(run_dir: &Path, sizes: &crate::flow::Sizes, force: bool) {
    if !on() {
        return;
    }
    let due = P.with(|p| {
        let mut p = p.borrow_mut();
        let now = Instant::now();
        let due = force || p.last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(30));
        if due {
            p.last = Some(now);
        }
        due
    });
    if due {
        let v = P.with(|p| report(&p.borrow(), sizes));
        let tmp = run_dir.join("profile.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        std::fs::rename(&tmp, run_dir.join("profile.json")).unwrap();
    }
}

fn report(p: &Prof, sizes: &crate::flow::Sizes) -> serde_json::Value {
    // Scope operators (an address that prefixes another's) hold their children's time.
    let mut scope_ops = std::collections::HashSet::new();
    for a in p.op_addr.values() {
        for k in 1..a.len() {
            if let Some(id) = p.addr_op.get(&a[..k]) {
                scope_ops.insert(*id);
            }
        }
    }
    let mut sent_by_op: FxHashMap<usize, i64> = FxHashMap::default();
    for (ch, n) in &p.sent_by_chan {
        if let Some((scope, src)) = p.chan.get(ch) {
            let mut addr = scope.clone();
            addr.push(*src);
            if let Some(id) = p.addr_op.get(&addr) {
                *sent_by_op.entry(*id).or_default() += n;
            }
        }
    }
    #[derive(Default)]
    struct Agg {
        secs: f64,
        sent: i64,
        arranged: isize,
        ops: Vec<(f64, i64, isize, String)>,
    }
    let mut per_node: BTreeMap<usize, Agg> = BTreeMap::new();
    let mut per_stage: BTreeMap<String, (f64, i64, isize)> = BTreeMap::new();
    let mut scopes = Vec::new();
    for (id, name) in &p.op_name {
        let secs = p.busy.get(id).map_or(0.0, |d| d.as_secs_f64());
        if scope_ops.contains(id) {
            scopes.push(json!({"op": id, "name": name, "secs": secs}));
            continue;
        }
        let sent = sent_by_op.get(id).copied().unwrap_or(0);
        let arranged = sizes.records.get(id).copied().unwrap_or(0);
        let Some(&ix) = p.op_node.get(id) else { continue };
        let a = per_node.entry(ix).or_default();
        a.secs += secs;
        a.sent += sent;
        a.arranged += arranged;
        a.ops.push((secs, sent, arranged, name.clone()));
        let s = per_stage.entry(format!("{}:{}", p.nodes[ix].graph, p.nodes[ix].label)).or_default();
        s.0 += secs;
        s.1 += sent;
        s.2 += arranged;
    }
    let mut nodes: Vec<(&usize, &Agg)> = per_node.iter().collect();
    nodes.sort_by(|a, b| b.1.secs.total_cmp(&a.1.secs));
    let nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|(ix, a)| {
            let n = &p.nodes[**ix];
            let mut ops = a.ops.clone();
            ops.sort_by(|x, y| y.0.total_cmp(&x.0));
            json!({"graph": n.graph, "node": n.node, "mode": n.mode, "kind": n.kind, "stage": n.label, "secs": a.secs, "sent": a.sent, "arranged": a.arranged,
                   "ops": ops.iter().take(6).map(|o| json!([o.3, o.0, o.1, o.2])).collect::<Vec<_>>()})
        })
        .collect();
    let mut stages: Vec<(&String, &(f64, i64, isize))> = per_stage.iter().collect();
    stages.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    let mut iters: BTreeMap<String, BTreeMap<String, Vec<(u64, i64, i64)>>> = BTreeMap::new();
    for ((f, arm, i), (u, d)) in &p.iters {
        iters.entry(f.clone()).or_default().entry(arm.clone()).or_default().push((*i, *u, *d));
    }
    let total: f64 = per_node.values().map(|a| a.secs).sum();
    json!({
        "busy_secs_total": total,
        "stages": stages.iter().map(|(s, v)| json!({"stage": s, "secs": v.0, "sent": v.1, "arranged": v.2})).collect::<Vec<_>>(),
        "nodes": nodes,
        "scopes": scopes,
        "iterations": iters,
        "notes": p.notes,
    })
}
