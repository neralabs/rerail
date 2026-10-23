//! `pattern-check`: the pattern front end's differential (INVARIANTS I3/I4). Every matcher input of
//! the policies that the leaf executor compiles and `rerail_facts::front::pattern` lowers is
//! evaluated three ways over each corpus file:
//!
//! - the leaf executor (`ast.rs` through `eval::Input::rows`, exact against native): the truth;
//! - the physical plan of the lowered algebra (`lowered.rs`, what `facts-a repo` runs), on every
//!   file;
//! - the reference evaluator over the lowered algebra and the content's core relations
//!   (`views::core_rows`, `string_value` included), on files of at most `--max-nodes` nodes (it
//!   evaluates correlated subqueries naively, O(nodes²) per relation search).
//!
//! Rows (values and order), anchors, witnesses and run-time refusals must be identical.
//!
//! `facts-a pattern-check --root DIR --out-dir DIR [--rules DIR] [--input NAME] [--threads N]
//! [--limit N] [--shard K/N] [--max-nodes N] [--config JSON]`
use crate::ast::{Compiler, FileMemo, Names};
use crate::eval::{Anchor as EAnchor, FileCtx, Input, Proj, Rows};
use crate::facts::{self, Facts};
use crate::sql::engine::Schema;
use crate::sql::value::{Aff, V};
use crate::{arg, leaf, lowered, views};
use rerail_facts::ir::{Anchor, CoreRel, FileSource, InputDef, InputKind, Projection, Rel};
use rerail_facts::reference::{Evaluator, Row};
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Case {
    rule: String,
    def: InputDef,
    utils: BTreeMap<String, Json>,
}

#[derive(Default, Clone, serde::Serialize)]
struct Stat {
    files: u64,
    rows: u64,
    refused_files: u64,
    /// The physical plan: `None` when the input does not plan (it runs on the leaf executor).
    physical: Option<String>,
    phys_files: u64,
    phys_mismatches: u64,
    ref_files: u64,
    ref_mismatches: u64,
    /// Evaluations whose matcher counts (`crate::counts::InputCounts`: nodes, roots, rows) were
    /// compared (leaf vs physical on every file, the reference too where it runs), and the
    /// differences.
    counts_compared: u64,
    count_mismatches: u64,
    examples: Vec<String>,
}

/// The lowered input's selection `Filter(file ⋈ node, keep)` (`front::pattern`), under its
/// projection and capture joins: its rows are the matcher's roots.
fn selection(r: &Rel<FileSource>) -> Option<&Rel<FileSource>> {
    match r {
        Rel::Filter { input, .. } if format!("{input:?}") == format!("{:?}", rerail_facts::front::pattern::nodes()) => Some(r),
        Rel::Project { input, .. } | Rel::Filter { input, .. } | Rel::Sort { input, .. } | Rel::Distinct { input, .. } => selection(input),
        Rel::Join { left, .. } => selection(left),
        _ => None,
    }
}

fn cases(rules: &str, config: &Map<String, Json>) -> (Vec<Case>, BTreeMap<String, Vec<String>>) {
    let mut paths: Vec<_> = std::fs::read_dir(rules).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    let mut out = Vec::new();
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut leaves = leaf::Leaves::new();
    for p in paths {
        let doc: Json = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let Ok(c) = rerail_facts::compile::compile_report(&doc, config, &mut leaves) else { continue };
        let resolved = rerail_facts::policy::resolve(&doc, config).expect("resolve");
        let mut utils = BTreeMap::new();
        for cl in resolved["clauses"].as_array().into_iter().flatten() {
            if let Some(u) = cl.get("define").and_then(|d| d.get("utils")).and_then(Json::as_object) {
                utils.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        for def in c.program.inputs {
            if !matches!(def.kind, InputKind::Pattern { .. }) {
                continue;
            }
            match &def.algebra {
                Ok(_) => out.push(Case { rule: c.program.rule.clone(), def, utils: utils.clone() }),
                Err(e) => refused.entry(e.clone()).or_default().push(format!("{}/{}", c.program.rule, def.name)),
            }
        }
    }
    (out, refused)
}

/// The leaf executor's input for a definition (projections and anchor mapped one to one).
fn leaf_input(c: &Case, names: &mut Names) -> Result<Input, String> {
    let InputKind::Pattern { query } = &c.def.kind else { unreachable!() };
    let mut comp = Compiler::new(names, &facts::GRAMMAR, &c.utils);
    let q = comp.query(query)?;
    fn proj(p: &Projection, names: &mut Names) -> Result<Proj, String> {
        Ok(match p {
            Projection::Path => Proj::Path,
            Projection::Directory => Proj::Directory,
            Projection::Name => Proj::Name,
            Projection::Stem => Proj::Stem,
            Projection::Capture(n) => Proj::Capture(names.id(n)),
            Projection::Text(n) => Proj::Text(names.id(n)),
            Projection::Kind(n) => Proj::Kind(names.id(n)),
            Projection::StartByte(n) => Proj::Start(names.id(n)),
            Projection::EndByte(n) => Proj::End(names.id(n)),
            Projection::StartLine(n) => Proj::Line(names.id(n)),
            Projection::Regex { source, regex, group } => Proj::Regex(Box::new(proj(source, names)?), crate::rx::get(regex).map_err(|e| e.to_string())?, *group as usize),
            other => return Err(format!("projection {other:?}")),
        })
    }
    let projs = c.def.fields.iter().map(|(_, p)| proj(p, names)).collect::<Result<Vec<_>, _>>()?;
    let anchor = match &c.def.anchor {
        Anchor::Span => EAnchor::Span,
        Anchor::FileStart => EAnchor::FileStart,
        Anchor::Capture(n) => EAnchor::Capture(names.id(n)),
        a => return Err(format!("anchor {a:?}")),
    };
    Ok(Input { query: q, fields: c.def.fields.iter().map(|(f, _)| f.clone()).collect(), projs, each: None, anchor, paths: None, node_regexes: Vec::new(), groups: Vec::new(), phys: Err("pattern-check".into()) })
}

/// The content's core relations, as the reference evaluator reads them (`views::core_rows`).
fn core(f: &Facts, path: &str) -> BTreeMap<CoreRel, Vec<Row>> {
    views::core_rows(f, path)
}

fn show(v: &V) -> String {
    format!("{v:?}")
}

fn table_rows(r: &Rows) -> Vec<Vec<String>> {
    (0..r.table.len()).map(|i| r.table.row(i).iter().map(show).collect()).collect()
}

fn note(st: &mut Stat, path: &str, what: String) {
    if st.examples.len() < 5 {
        st.examples.push(format!("{path}: {what}"));
    }
}

pub fn run(args: &[String]) {
    let root = arg(args, "--root").expect("--root");
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let rules = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let only = arg(args, "--input");
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    let max_nodes: usize = arg(args, "--max-nodes").map_or(usize::MAX, |x| x.parse().unwrap());
    let config: Map<String, Json> = arg(args, "--config").map(|c| serde_json::from_str(&c).expect("--config")).unwrap_or_default();
    std::fs::create_dir_all(&out_dir).unwrap();
    let root = Path::new(&root);
    let mut paths = crate::repo::walk(root);
    if let Some(l) = limit {
        paths.truncate(l);
    }
    // `--shard k/n`: every n-th file from the k-th (runs stay under the 15-minute limit).
    let shard = arg(args, "--shard");
    if let Some(s) = &shard {
        let (k, n) = s.split_once('/').map(|(k, n)| (k.parse::<usize>().unwrap(), n.parse::<usize>().unwrap())).expect("--shard k/n");
        paths = paths.into_iter().enumerate().filter(|(i, _)| i % n == k).map(|(_, p)| p).collect();
    }
    let (all, refused) = cases(&rules, &config);
    let names: Vec<String> = all.iter().map(|c| format!("{}/{}", c.rule, c.def.name)).collect();
    let keep: Vec<usize> = (0..all.len()).filter(|&i| only.as_ref().is_none_or(|o| names[i].contains(o.as_str()))).collect();
    eprintln!("pattern-check: {} lowered inputs ({} selected), {} files", all.len(), keep.len(), paths.len());
    let next = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let outs: Vec<Vec<Stat>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                let (paths, next, keep, skipped, rules, config) = (&paths, &next, &keep, &skipped, &rules, &config);
                s.spawn(move || {
                    // Per thread: the inputs (the leaf executor's values are `Rc`-based) and their
                    // physical plans, all over one predicate table.
                    let (cs, _) = cases(rules, config);
                    let mut names = Names::default();
                    let leafs: Vec<Option<Input>> = keep.iter().map(|&i| leaf_input(&cs[i], &mut names).ok()).collect();
                    let mut preds = lowered::Preds::default();
                    let plans: Vec<Result<lowered::Plan, String>> = keep.iter().map(|&i| lowered::plan(cs[i].def.algebra.as_ref().unwrap(), &mut preds)).collect();
                    let mut stats: Vec<Stat> = plans.iter().map(|p| Stat { physical: Some(p.as_ref().err().cloned().unwrap_or_else(|| "planned".into())), ..Stat::default() }).collect();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(i) else { break };
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let source = Arc::new(src);
                        let Ok(facts::Built::Facts(f)) = facts::build(path, source.clone()) else { continue };
                        let small = f.len() <= max_nodes;
                        if !small {
                            skipped.fetch_add(1, Ordering::Relaxed);
                        }
                        let core: Option<BTreeMap<CoreRel, std::rc::Rc<Vec<Row>>>> = if small { Some(core(&f, path).into_iter().map(|(k, v)| (k, std::rc::Rc::new(v))).collect()) } else { None };
                        let scan = |s: &FileSource| -> Result<Vec<Row>, String> {
                            match s {
                                FileSource::Core(c) => Ok((*core.as_ref().unwrap()[c]).clone()),
                                o => Err(format!("{o:?} in a lowered input")),
                            }
                        };
                        // Core relations are lent by reference (not copied per correlated scan).
                        let lend = |s: &FileSource| -> Option<std::rc::Rc<Vec<Row>>> {
                            match s {
                                FileSource::Core(c) => core.as_ref().map(|m| m[c].clone()),
                                _ => None,
                            }
                        };
                        let ev = Evaluator::with_shared_content(&scan, &lend, path, &source);
                        let ctx = FileCtx { f: &f, path, source: &source };
                        let rc: std::rc::Rc<str> = std::rc::Rc::from(source.as_str());
                        let memo = FileMemo::default();
                        let mut phys_state = lowered::FileState::new(&f, path);
                        for (k, &ci) in keep.iter().enumerate() {
                            let Some(input) = &leafs[k] else { continue };
                            let c = &cs[ci];
                            let lowered = c.def.algebra.as_ref().unwrap();
                            let schema = Schema { cols: c.def.schema.iter().map(|c| c.name.clone()).collect(), affs: vec![Aff::None; c.def.schema.len()] };
                            let a = input.rows(&ctx, &schema, &memo, &rc);
                            let st = &mut stats[k];
                            st.files += 1;
                            st.rows += a.table.len() as u64;
                            if a.unsupported.is_some() {
                                st.refused_files += 1;
                            }
                            let rows_a = table_rows(&a);
                            // The physical plan, on every file.
                            if let Some(b) = plans[k].as_ref().ok().and_then(|plan| lowered::rows(plan, &preds, &mut phys_state, &schema, &rc)) {
                                st.phys_files += 1;
                                let (ca, cb) = (&a.counts, &b.counts);
                                if a.unsupported.is_none() && b.unsupported.is_none() {
                                    st.counts_compared += 1;
                                    if (ca.nodes, ca.roots, ca.rows) != (cb.nodes, cb.roots, cb.rows) {
                                        st.count_mismatches += 1;
                                        note(st, path, format!("counts leaf {:?} vs physical {:?}", (ca.nodes, ca.roots, ca.rows), (cb.nodes, cb.roots, cb.rows)));
                                    }
                                }
                                let why = if a.unsupported != b.unsupported {
                                    Some(format!("refusal leaf {:?} vs physical {:?}", a.unsupported, b.unsupported))
                                } else if a.unsupported.is_some() {
                                    None
                                } else if rows_a.len() != b.table.len() {
                                    Some(format!("{} rows vs physical {}", rows_a.len(), b.table.len()))
                                } else if let Some((r, (x, y))) = rows_a.iter().zip(table_rows(&b)).enumerate().find(|(_, (x, y))| **x != *y) {
                                    Some(format!("row {r}: {x:?} vs physical {y:?}"))
                                } else if a.anchors != b.anchors || a.witnesses != b.witnesses {
                                    Some("anchors or witnesses differ from the physical plan's".into())
                                } else {
                                    None
                                };
                                if let Some(w) = why {
                                    st.phys_mismatches += 1;
                                    note(st, path, w);
                                }
                            }
                            if !small {
                                continue;
                            }
                            // The reference: the first non-empty refusal, else the rows.
                            st.ref_files += 1;
                            let mut b_refusal = None;
                            for (r, msg) in &lowered.refuse_if {
                                match ev.rel(r, &[]) {
                                    Ok(rows) if !rows.is_empty() => {
                                        b_refusal = Some(msg.clone());
                                        break;
                                    }
                                    Ok(_) => {}
                                    Err(e) => b_refusal = Some(format!("reference error: {e}")),
                                }
                            }
                            let why = match (&a.unsupported, &b_refusal) {
                                (Some(x), Some(y)) if x == y => None,
                                (None, None) => match ev.rel(&lowered.rows, &[]) {
                                    Err(e) => Some(format!("reference error: {e}")),
                                    Ok(rows_b) => {
                                        // The reference's matcher counts: the `node` rows the selection ranges over,
                                        // the selection's rows, the input's rows.
                                        let roots = selection(&lowered.rows).map(|s| ev.rel(s, &[]).map(|r| r.len() as u64));
                                        let nodes = core.as_ref().map_or(0, |m| m[&CoreRel::Node].len() as u64);
                                        st.counts_compared += 1;
                                        let want = (a.counts.nodes, a.counts.roots, a.counts.rows);
                                        if !matches!(roots, Some(Ok(r)) if (nodes, r, rows_b.len() as u64) == want) {
                                            st.count_mismatches += 1;
                                            note(st, path, format!("counts leaf {want:?} vs reference nodes {nodes} roots {roots:?} rows {}", rows_b.len()));
                                        }
                                        let rows_b: Vec<Vec<String>> = rows_b.iter().map(|r| r.iter().map(show).collect()).collect();
                                        if rows_a.len() != rows_b.len() {
                                            Some(format!("{} rows vs reference {}", rows_a.len(), rows_b.len()))
                                        } else {
                                            rows_a.iter().zip(&rows_b).enumerate().find(|(_, (x, y))| x != y).map(|(r, (x, y))| format!("row {r}: {x:?} vs reference {y:?}"))
                                        }
                                    }
                                },
                                (x, y) => Some(format!("refusal leaf {x:?} vs reference {y:?}")),
                            };
                            if let Some(w) = why {
                                st.ref_mismatches += 1;
                                note(st, path, w);
                            }
                        }
                    }
                    stats
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut total: Vec<Stat> = vec![Stat::default(); keep.len()];
    for o in outs {
        for (t, s) in total.iter_mut().zip(o) {
            t.physical = s.physical.clone();
            t.files += s.files;
            t.rows += s.rows;
            t.refused_files += s.refused_files;
            t.phys_files += s.phys_files;
            t.phys_mismatches += s.phys_mismatches;
            t.ref_files += s.ref_files;
            t.ref_mismatches += s.ref_mismatches;
            t.counts_compared += s.counts_compared;
            t.count_mismatches += s.count_mismatches;
            t.examples.extend(s.examples);
        }
    }
    let per: BTreeMap<String, Stat> = keep.iter().zip(&total).map(|(&i, s)| (names[i].clone(), s.clone())).collect();
    let phys_mismatches: u64 = total.iter().map(|s| s.phys_mismatches).sum();
    let ref_mismatches: u64 = total.iter().map(|s| s.ref_mismatches).sum();
    let counts_compared: u64 = total.iter().map(|s| s.counts_compared).sum();
    let count_mismatches: u64 = total.iter().map(|s| s.count_mismatches).sum();
    let rows: u64 = total.iter().map(|s| s.rows).sum();
    let planned = total.iter().filter(|s| s.physical.as_deref() == Some("planned")).count();
    let mut not_planned: BTreeMap<String, u64> = BTreeMap::new();
    for s in &total {
        if let Some(p) = s.physical.as_deref().filter(|p| *p != "planned") {
            *not_planned.entry(p.to_owned()).or_default() += 1;
        }
    }
    let report = serde_json::json!({
        "lowered_inputs": all.len(),
        "checked_inputs": keep.len(),
        "planned_inputs": planned,
        "files": paths.len(),
        "shard": shard,
        "reference_skipped_over_max_nodes": skipped.load(Ordering::Relaxed),
        "rows": rows,
        "phys_mismatches": phys_mismatches,
        "ref_mismatches": ref_mismatches,
        "counts_compared": counts_compared,
        "count_mismatches": count_mismatches,
        "not_lowered": refused,
        "not_planned": not_planned,
        "per_input": per,
    });
    std::fs::write(Path::new(&out_dir).join("pattern-check.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!(
        "pattern-check: {} inputs ({} planned), {} files, {} rows, {} physical mismatches, {} reference mismatches ({} files over --max-nodes skipped by the reference); counts {} compared, {} mismatches",
        keep.len(),
        planned,
        paths.len(),
        rows,
        phys_mismatches,
        ref_mismatches,
        skipped.load(Ordering::Relaxed),
        counts_compared,
        count_mismatches
    );
}
