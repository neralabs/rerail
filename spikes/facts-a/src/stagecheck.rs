//! `facts-a stage-check` (the gate's `a-vs-reference`, INVARIANTS I1): the engine's relational
//! stages against `rerail_facts::reference`, which defines truth.
//!
//! Two sources of runs: every recorded call of `--recording` files (each rule compiled under the
//! call's configuration and scope and evaluated as `facts-a replay` does: the recordings exercise
//! every clause on small inputs, repository clauses included), and optionally the files under
//! `--root` with the suite's production configuration, as `facts-a repo` runs them (per-file
//! `eval::eval_file`, `eval::finish` for repository partitions). `rel::TRACE` hands over every
//! table a clause's `rel::run` read (inputs, syntax tables) and stored (stages). The same clause,
//! lowered by the front end (`rerail_facts::front::rule::front`, before classification and
//! rewrites), is evaluated stage by stage by `reference::evaluate_stages` over the engine's input
//! and syntax tables; each stage's own result feeds only the reference's later stages, so a wrong
//! engine stage cannot hide a wrong reference stage or the reverse. Per stage:
//!
//! - rows as multisets of typed values (an integer and a real are different values; affinities
//!   are applied on both sides as CTAS stores them);
//! - order, when the stage's query is an `ORDER BY` whose keys reach the output unchanged: the
//!   sequences of sort-key values are equal (ties may be in any order, as SQLite leaves them);
//! - not compared, counted by reason: a stage the engine did not store (a kernel absorbed it, or
//!   the engine failed: its failure is reported by `repo`), a stage the reference refuses (by
//!   name), and per-file runs over files with more than `--max-nodes` nodes, or repository runs
//!   whose input tables hold more than `--max-input-rows` rows in total (the reference's joins are
//!   nested loops).
//!
//! `stage-check --out-dir DIR [--recording R]… [--root DIR [--checks a,b] [--sample k/n]
//! [--threads N]] [--max-nodes N] [--max-input-rows N]`. Output `results.json`; exit 0 (the gate
//! decides).
use crate::{arg, call_config, eval, facts, policy_doc, rel};
use rerail_facts::front::rule::{ClauseBody, front};
use rerail_facts::ir::{AnySource, Rel, Stage, StageDef};
use rerail_facts::reference::{self, Row};
use rerail_facts::sql::value::V;
use serde_json::{Value as Json, json};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// One relational clause as the front end lowers it.
struct ClauseIr {
    stages: Vec<Stage<AnySource>>,
    /// Lower-case table name per front-end input id.
    inputs: Vec<String>,
    /// Lower-case names of the syntax spec's node, edge and ownership tables.
    syntax: [Option<String>; 3],
}

#[derive(Default)]
struct Stats {
    runs: u64,
    stages_compared: u64,
    rows_compared: u64,
    ordered_compared: u64,
    mismatches: Vec<Json>,
    mismatch_total: u64,
    not_compared: BTreeMap<String, u64>,
    /// Per `check/clause`: (runs, reference seconds).
    per_clause: BTreeMap<String, (u64, f64)>,
}

impl Stats {
    fn skip(&mut self, why: impl Into<String>) {
        *self.not_compared.entry(why.into()).or_default() += 1;
    }

    fn merge(&mut self, o: Stats) {
        self.runs += o.runs;
        self.stages_compared += o.stages_compared;
        self.rows_compared += o.rows_compared;
        self.ordered_compared += o.ordered_compared;
        self.mismatch_total += o.mismatch_total;
        self.mismatches.extend(o.mismatches);
        for (k, v) in o.not_compared {
            *self.not_compared.entry(k).or_default() += v;
        }
        for (k, (n, s)) in o.per_clause {
            let e = self.per_clause.entry(k).or_default();
            e.0 += n;
            e.1 += s;
        }
    }
}

/// A value with its storage class: `1` and `1.0` differ.
fn tkey(v: &V) -> String {
    match v {
        V::Null => "n".into(),
        V::Int(i) => format!("i{i}"),
        V::Real(f) => format!("r{}", f.to_bits()),
        V::Text(t) => format!("t{}:{t}", t.len()),
    }
}

fn row_key(r: &[V]) -> String {
    r.iter().map(tkey).collect::<Vec<_>>().join("\u{1f}")
}

fn bag(rows: &[Row]) -> HashMap<String, i64> {
    let mut m = HashMap::new();
    for r in rows {
        *m.entry(row_key(r)).or_default() += 1;
    }
    m
}

/// One rule's relational clauses as the front end lowers them under `config`, by clause id.
fn lower(doc: &Json, config: &serde_json::Map<String, Json>) -> BTreeMap<String, Rc<ClauseIr>> {
    let mut out = BTreeMap::new();
    let mut leaves = crate::leaf::Leaves::new();
    let Ok(f) = front(doc, config, &mut leaves) else { return out };
    let inputs: Vec<String> = f.inputs.iter().map(|i| i.name.to_ascii_lowercase()).collect();
    for c in f.clauses {
        let Some(ClauseBody::Relational { syntax, stages, .. }) = c.body else { continue };
        let names = syntax.as_ref().map(|s| ["name", "edges", "ownership"].map(|k| s.spec.get(k).and_then(Json::as_str).map(str::to_ascii_lowercase)));
        out.insert(c.id, Rc::new(ClauseIr { stages, inputs: inputs.clone(), syntax: names.unwrap_or([None, None, None]) }));
    }
    out
}

/// Compares one traced `rel::run` with the reference.
fn compare(ir: &ClauseIr, label: &str, path: &str, traced: &rel::Traced, st: &mut Stats) {
    let table = |name: &str| traced.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone());
    let rows_of = |t: &crate::sql::engine::Table| -> Vec<Row> { (0..t.len()).map(|i| t.row(i)).collect() };
    let scan = |s: &AnySource| -> Result<Vec<Row>, String> {
        let name = match s {
            AnySource::Input(i) => ir.inputs.get(*i as usize).cloned(),
            AnySource::Syntax(t) => ir.syntax[*t as usize].clone(),
            AnySource::ReferenceEdges => return Err("reference_edges (the cross-file graph) is not traced".into()),
            AnySource::Stage(_) => unreachable!("stages resolve first"),
        };
        let name = name.ok_or_else(|| format!("{s:?} has no table name"))?;
        match table(&name) {
            Some(Some(t)) => Ok(rows_of(&t)),
            _ => Err(format!("table {name} not read by the engine")),
        }
    };
    let t0 = Instant::now();
    let (done, refusal) = reference::evaluate_stages(&ir.stages, |s| if let AnySource::Stage(i) = s { Some(*i) } else { None }, &scan);
    let e = st.per_clause.entry(label.to_owned()).or_default();
    e.0 += 1;
    e.1 += t0.elapsed().as_secs_f64();
    st.runs += 1;
    if let Some(r) = refusal {
        let at = ir.stages.get(done.len()).map_or("?", |s| s.name.as_str());
        st.skip(format!("reference refused ({label} stage {at}): {}", r.chars().take(120).collect::<String>()));
    }
    for (stage, want) in ir.stages.iter().zip(&done) {
        let got = match table(&stage.name.to_ascii_lowercase()) {
            Some(Some(t)) => rows_of(&t),
            Some(None) => {
                st.skip("stage absorbed by a kernel (not stored)");
                continue;
            }
            None => {
                st.skip("engine stopped before the stage (its failure is reported by repo)");
                continue;
            }
        };
        st.stages_compared += 1;
        st.rows_compared += want.len() as u64;
        let (bg, bw) = (bag(&got), bag(want));
        let mut problem = None;
        if bg != bw {
            let only = |a: &HashMap<String, i64>, b: &HashMap<String, i64>| -> Vec<String> {
                a.iter().filter(|(k, n)| b.get(*k).copied().unwrap_or(0) < **n).take(3).map(|(k, n)| format!("{}×{}", n - b.get(k).copied().unwrap_or(0), k.replace('\u{1f}', " | "))).collect()
            };
            problem = Some(json!({"engine_rows": got.len(), "reference_rows": want.len(), "only_engine": only(&bg, &bw), "only_reference": only(&bw, &bg)}));
        } else if let Some(cols) = sort_cols(&stage.def) {
            st.ordered_compared += 1;
            let seq = |rows: &[Row]| -> Vec<String> { rows.iter().map(|r| cols.iter().map(|c| r.get(*c).map_or("-".into(), tkey)).collect::<Vec<_>>().join("\u{1f}")).collect() };
            let (sg, sw) = (seq(&got), seq(want));
            if let Some(i) = sg.iter().zip(&sw).position(|(a, b)| a != b) {
                problem = Some(json!({"order": format!("sort keys differ at row {i}: engine {} vs reference {}", sg[i], sw[i])}));
            }
        }
        if let Some(mut p) = problem {
            st.mismatch_total += 1;
            if st.mismatches.len() < 50 {
                p["clause"] = json!(label);
                p["stage"] = json!(stage.name);
                p["path"] = json!(path);
                st.mismatches.push(p);
            }
        }
    }
}

/// The output columns holding a stage's `ORDER BY` keys, when its query is a top-level `Sort`, or
/// a `Sort` under a `Project` that passes every key column through unchanged; `None` when the
/// stage specifies no order (or the keys are computed, then compared as a multiset only).
fn sort_cols(def: &StageDef<AnySource>) -> Option<Vec<usize>> {
    let StageDef::Query { rel } = def else { return None };
    match rel {
        Rel::Sort { keys, .. } => Some(keys.iter().map(|(c, _)| *c as usize).collect()),
        Rel::Project { input, exprs, .. } => {
            let Rel::Sort { keys, .. } = &**input else { return None };
            keys.iter().map(|(c, _)| exprs.iter().position(|e| matches!(e, rerail_facts::ir::Expr::Col { depth: 0, col } if col == c))).collect()
        }
        _ => None,
    }
}

/// Per compiled `RelPlan` (by address): `check/clause`, whether it is per file, and its lowering
/// (`None`: the front end refuses the clause).
type Registry = Rc<RefCell<HashMap<usize, (String, bool, Option<Rc<ClauseIr>>)>>>;

/// Adds `rule`'s relational clauses, compiled under `config`, to the registry.
fn register(reg: &Registry, rule: &eval::Rule, doc: &Json, config: &serde_json::Map<String, Json>) {
    let irs = lower(doc, config);
    for c in &rule.clauses {
        if let eval::Kind::Relational(p) = &c.kind {
            reg.borrow_mut().insert(&**p as *const rel::RelPlan as usize, (format!("{}/{}", rule.check, c.id), p.per_file, irs.get(&c.id).cloned()));
        }
    }
}

/// Installs the trace hook on this thread: traced runs of registered clauses are compared into the
/// returned stats. `nodes` holds the current file's node count (0 in `finish`).
fn install(reg: Registry, nodes: Rc<Cell<usize>>, max_nodes: usize, max_rows: usize) -> Rc<RefCell<Stats>> {
    let stats = Rc::new(RefCell::new(Stats::default()));
    let st = stats.clone();
    rel::TRACE.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |plan: &rel::RelPlan, path: &str, traced: &rel::Traced| {
            let mut st = st.borrow_mut();
            let reg = reg.borrow();
            let Some((label, per_file, ir)) = reg.get(&(plan as *const rel::RelPlan as usize)) else {
                st.skip("run of an unregistered plan");
                return;
            };
            let Some(ir) = ir else {
                st.skip(format!("front end refuses the clause ({label})"));
                return;
            };
            if *per_file && nodes.get() > max_nodes {
                st.skip("per-file run over --max-nodes");
                return;
            }
            let input_rows: usize = traced.iter().filter_map(|(_, t)| t.as_ref()).take(plan.inputs.len()).map(|t| t.len()).sum();
            if !*per_file && input_rows > max_rows {
                st.skip(format!("repository run over --max-input-rows ({label}: {input_rows} input rows)"));
                return;
            }
            compare(ir, label, path, traced, &mut st);
        }));
    });
    stats
}

fn uninstall(stats: Rc<RefCell<Stats>>) -> Stats {
    rel::TRACE.with(|h| *h.borrow_mut() = None);
    Rc::try_unwrap(stats).ok().expect("hook dropped").into_inner()
}

/// Every recorded call of `recordings` (as `facts-a replay` selects them, every check): its rule
/// compiled under the call's configuration and scope, its files evaluated as `replay` does.
fn recordings(recs: &[String], policies: &str, max_nodes: usize, max_rows: usize) -> (Stats, u64) {
    let reg: Registry = Default::default();
    let stats = install(reg.clone(), Rc::new(Cell::new(0)), max_nodes, max_rows);
    let mut compiled: HashMap<String, Option<Rc<Vec<eval::Rule>>>> = HashMap::new();
    let mut docs: HashMap<String, Json> = HashMap::new();
    let mut calls = 0;
    for rec in recs {
        let d: Json = serde_json::from_str(&std::fs::read_to_string(rec).unwrap()).unwrap();
        for e in d["events"].as_array().unwrap() {
            if e.get("retained_reason").is_some() || e.get("upstream_passed") == Some(&Json::Bool(false)) || e.get("files").is_none() {
                continue;
            }
            let check = e["check"].as_str().unwrap().to_owned();
            let doc = docs.entry(check.clone()).or_insert_with(|| policy_doc(policies, &check)).clone();
            let (config, scope) = call_config(&doc, e);
            let key = format!("{check}\0{}\0{}", serde_json::to_string(&config).unwrap(), serde_json::to_string(&scope).unwrap());
            let rules = compiled.entry(key).or_insert_with(|| {
                let r = crate::caught(|| eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR)).ok()?.ok()?;
                register(&reg, &r, &doc, &config);
                Some(Rc::new(vec![r]))
            });
            let Some(rules) = rules.clone() else { continue };
            let files: Vec<(String, Arc<String>)> = e["files"].as_array().unwrap().iter().map(|f| (f["path"].as_str().unwrap().to_owned(), Arc::new(f["source"].as_str().unwrap().to_owned()))).collect();
            crate::evaluate(&rules, &files);
            calls += 1;
        }
    }
    (uninstall(stats), calls)
}

/// Every file under `root` with the suite's production configuration, as `facts-a repo` runs it.
fn corpus(root: &Path, policies: &str, checks: &[String], threads: usize, sample: (usize, usize), max_nodes: usize, max_rows: usize) -> (Stats, usize, usize) {
    let event = json!({"production_scope": true});
    let docs: Vec<(Json, serde_json::Map<String, Json>)> = checks.iter().map(|c| {
        let doc = policy_doc(policies, c);
        let (config, _) = call_config(&doc, &event);
        (doc, config)
    }).collect();
    // `--sample k/n`: every n-th file of the sorted walk from the k-th (deterministic).
    let paths: Vec<String> = crate::repo::walk(root).into_iter().enumerate().filter(|(i, _)| i % sample.1 == sample.0).map(|(_, p)| p).collect();
    let (rules, _refused) = crate::repo::compile_checks(policies, checks);
    let rules: Arc<[eval::Rule]> = rules.into();
    // The rules that compiled, with their documents (compile_checks keeps the order of `checks`).
    let register_all = |reg: &Registry| {
        for r in rules.iter() {
            if let Some(i) = checks.iter().position(|c| *c == r.check) {
                register(reg, r, &docs[i].0, &docs[i].1);
            }
        }
    };
    let next = AtomicUsize::new(0);
    // Repository-partition rows spill to disk as in `facts-a repo` (their values hold `Rc`s).
    let spill_dir = std::env::temp_dir().join(format!("facts-a-stagecheck-{}", std::process::id()));
    std::fs::create_dir_all(&spill_dir).unwrap();
    let workers: Vec<(Stats, eval::BudgetRows)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|wi| {
                let (rules, paths, next, spill_dir, register_all) = (&*rules, &paths, &next, &spill_dir, &register_all);
                s.spawn(move || {
                    let reg: Registry = Default::default();
                    register_all(&reg);
                    let nodes = Rc::new(Cell::new(0));
                    let stats = install(reg, nodes.clone(), max_nodes, max_rows);
                    let mut out = eval::Output::default();
                    let mut budget = BTreeMap::new();
                    let mut sink = crate::spill::Spill::new(spill_dir, wi);
                    while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let Ok(facts::Built::Facts(f)) = facts::build(path, Arc::new(src.clone())) else { continue };
                        nodes.set(f.len());
                        let ctx = eval::FileCtx { f: &f, path, source: &src };
                        eval::eval_file(rules, &ctx, &mut out, &mut sink, &mut budget);
                    }
                    sink.finish();
                    (uninstall(stats), budget)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut stats = Stats::default();
    let mut budget: eval::BudgetRows = BTreeMap::new();
    for (s, b) in workers {
        stats.merge(s);
        for (k, v) in b {
            budget.entry(k).or_default().extend(v);
        }
    }
    let reg: Registry = Default::default();
    register_all(&reg);
    let main_stats = install(reg, Rc::new(Cell::new(0)), max_nodes, max_rows);
    eval::finish(&rules, |k| crate::spill::load(&spill_dir, threads, k), budget, &mut eval::Output::default());
    std::fs::remove_dir_all(&spill_dir).unwrap();
    stats.merge(uninstall(main_stats));
    (stats, paths.len(), rules.len())
}

pub fn run(args: &[String]) {
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let max_nodes: usize = arg(args, "--max-nodes").map_or(usize::MAX, |n| n.parse().unwrap());
    let max_rows: usize = arg(args, "--max-input-rows").map_or(usize::MAX, |n| n.parse().unwrap());
    let sample = arg(args, "--sample").map_or((0, 1), |s| s.split_once('/').map(|(k, n)| (k.parse().unwrap(), n.parse().unwrap())).expect("--sample k/n"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let t_all = Instant::now();
    let recs = crate::args_all(args, "--recording");
    let (rec_stats, calls) = recordings(&recs, &policies, max_nodes, max_rows);
    let rec_s = t_all.elapsed().as_secs_f64();
    let t_corpus = Instant::now();
    let (corpus_stats, files, rules) = match arg(args, "--root") {
        Some(root) => corpus(Path::new(&root), &policies, &crate::repo::selected_checks(args), threads, sample, max_nodes, max_rows),
        None => (Stats::default(), 0, 0),
    };
    let corpus_s = t_corpus.elapsed().as_secs_f64();
    let part = |s: &Stats, secs: f64| {
        let mut slowest: Vec<(&String, &(u64, f64))> = s.per_clause.iter().collect();
        slowest.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        json!({
            "runs": s.runs,
            "stages_compared": s.stages_compared,
            "rows_compared": s.rows_compared,
            "ordered_compared": s.ordered_compared,
            "clauses_compared": s.per_clause.len(),
            "mismatch_total": s.mismatch_total,
            "mismatches": s.mismatches,
            "not_compared": s.not_compared,
            "reference_seconds_by_clause": slowest.iter().take(20).map(|(k, (n, t))| json!([k, n, t])).collect::<Vec<_>>(),
            "wall_s": secs,
        })
    };
    let res = json!({
        "recordings": recs, "calls": calls, "recorded": part(&rec_stats, rec_s),
        "files": files, "rules": rules, "sample": format!("{}/{}", sample.0, sample.1), "corpus": part(&corpus_stats, corpus_s),
        "max_nodes": (max_nodes != usize::MAX).then_some(max_nodes),
        "max_input_rows": (max_rows != usize::MAX).then_some(max_rows),
        "mismatch_total": rec_stats.mismatch_total + corpus_stats.mismatch_total,
        "stages_compared": rec_stats.stages_compared + corpus_stats.stages_compared,
        "wall_s": t_all.elapsed().as_secs_f64(),
    });
    std::fs::write(Path::new(&out_dir).join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    for (what, s) in [("recorded calls", &rec_stats), ("corpus", &corpus_stats)] {
        println!(
            "stage-check {what}: {} runs, {} stages compared ({} ordered, {} rows) in {} clauses, {} mismatches, {} not compared",
            s.runs, s.stages_compared, s.ordered_compared, s.rows_compared, s.per_clause.len(), s.mismatch_total, s.not_compared.values().sum::<u64>()
        );
    }
}
