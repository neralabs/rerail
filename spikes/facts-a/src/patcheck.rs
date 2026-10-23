//! `pattern-check`: the pattern front end's differential (INVARIANTS I3/I4). Every matcher input of
//! the policies that the leaf executor compiles and `rerail_facts::front::pattern` lowers is
//! evaluated both ways over each corpus file: the leaf executor (`ast.rs` through
//! `eval::Input::rows`, exact against native) and the reference evaluator over the lowered
//! algebra and the content's core relations (`views::core_tables`, plus `string_value` and
//! `string_value_unknown` from `pyvalue::decode`). Rows (values and order) and run-time refusals
//! must be identical.
//!
//! `facts-a pattern-check --root DIR --out-dir DIR [--rules DIR] [--input NAME] [--threads N]
//! [--limit N] [--max-nodes N] [--config JSON]`
use crate::ast::{Compiler, FileMemo, Names};
use crate::eval::{Anchor as EAnchor, FileCtx, Input, Proj};
use crate::facts::{self, Facts};
use crate::sql::engine::Schema;
use crate::sql::value::{Aff, V};
use crate::{arg, leaf, views};
use rerail_facts::ir::{Anchor, CoreRel, FileSource, InputDef, InputKind, Projection};
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
    mismatches: u64,
    examples: Vec<String>,
}

fn cases(rules: &str, config: &Map<String, Json>) -> (Vec<Case>, BTreeMap<String, u64>) {
    let mut paths: Vec<_> = std::fs::read_dir(rules).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    let mut out = Vec::new();
    let mut refused: BTreeMap<String, u64> = BTreeMap::new();
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
                Err(e) => *refused.entry(e.clone()).or_default() += 1,
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
    Ok(Input { query: q, fields: c.def.fields.iter().map(|(f, _)| f.clone()).collect(), projs, each: None, anchor, paths: None, node_regexes: Vec::new() })
}

/// The content's core relations, as the reference evaluator reads them (`views::core_rows`).
fn core(f: &Facts, path: &str) -> BTreeMap<CoreRel, Vec<Row>> {
    views::core_rows(f, path)
}

fn show(v: &V) -> String {
    format!("{v:?}")
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
                    // Per thread: the inputs (the leaf executor's values are `Rc`-based).
                    let (cs, _) = cases(rules, config);
                    let mut names = Names::default();
                    let leafs: Vec<Option<Input>> = keep.iter().map(|&i| leaf_input(&cs[i], &mut names).ok()).collect();
                    let mut stats = vec![Stat::default(); keep.len()];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(i) else { break };
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let source = Arc::new(src);
                        let Ok(facts::Built::Facts(f)) = facts::build(path, source.clone()) else { continue };
                        if f.len() > max_nodes {
                            skipped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let core = core(&f, path);
                        let scan = |s: &FileSource| -> Result<Vec<Row>, String> {
                            match s {
                                FileSource::Core(c) => Ok(core[c].clone()),
                                o => Err(format!("{o:?} in a lowered input")),
                            }
                        };
                        let ev = Evaluator::with_content(&scan, path, &source);
                        let ctx = FileCtx { f: &f, path, source: &source };
                        let rc: std::rc::Rc<str> = std::rc::Rc::from(source.as_str());
                        for (k, &ci) in keep.iter().enumerate() {
                            let Some(input) = &leafs[k] else { continue };
                            let c = &cs[ci];
                            let lowered = c.def.algebra.as_ref().unwrap();
                            let schema = Schema { cols: c.def.schema.iter().map(|c| c.name.clone()).collect(), affs: vec![Aff::None; c.def.schema.len()] };
                            let memo = FileMemo::default();
                            let a = input.rows(&ctx, &schema, &memo, &rc);
                            let st = &mut stats[k];
                            st.files += 1;
                            // The lowered side: the first non-empty refusal, else the rows.
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
                            let mismatch = |st: &mut Stat, what: String| {
                                st.mismatches += 1;
                                if st.examples.len() < 5 {
                                    st.examples.push(format!("{path}: {what}"));
                                }
                            };
                            match (&a.unsupported, &b_refusal) {
                                (Some(x), Some(y)) if x == y => {
                                    st.refused_files += 1;
                                    continue;
                                }
                                (None, None) => {}
                                (x, y) => {
                                    mismatch(st, format!("refusal leaf {x:?} vs lowered {y:?}"));
                                    continue;
                                }
                            }
                            let rows_b = match ev.rel(&lowered.rows, &[]) {
                                Ok(r) => r,
                                Err(e) => {
                                    mismatch(st, format!("reference error: {e}"));
                                    continue;
                                }
                            };
                            let rows_a: Vec<Vec<V>> = (0..a.table.len()).map(|r| a.table.row(r)).collect();
                            st.rows += rows_a.len() as u64;
                            if rows_a.len() != rows_b.len() {
                                mismatch(st, format!("{} rows vs {}", rows_a.len(), rows_b.len()));
                                continue;
                            }
                            if let Some((r, (x, y))) = rows_a.iter().zip(&rows_b).enumerate().find(|(_, (x, y))| x.iter().map(show).collect::<Vec<_>>() != y.iter().map(show).collect::<Vec<_>>()) {
                                mismatch(st, format!("row {r}: {x:?} vs {y:?}"));
                            }
                        }
                    }
                    stats
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut total = vec![Stat::default(); keep.len()];
    for o in outs {
        for (t, s) in total.iter_mut().zip(o) {
            t.files += s.files;
            t.rows += s.rows;
            t.refused_files += s.refused_files;
            t.mismatches += s.mismatches;
            t.examples.extend(s.examples);
        }
    }
    let per: BTreeMap<String, Stat> = keep.iter().zip(&total).map(|(&i, s)| (names[i].clone(), s.clone())).collect();
    let mismatches: u64 = total.iter().map(|s| s.mismatches).sum();
    let rows: u64 = total.iter().map(|s| s.rows).sum();
    let report = serde_json::json!({
        "lowered_inputs": all.len(),
        "checked_inputs": keep.len(),
        "files": paths.len(),
        "skipped_over_max_nodes": skipped.load(Ordering::Relaxed),
        "rows": rows,
        "mismatches": mismatches,
        "not_lowered": refused,
        "per_input": per,
    });
    std::fs::write(Path::new(&out_dir).join("pattern-check.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("pattern-check: {} inputs, {} files, {} rows, {} mismatches", keep.len(), paths.len(), rows, mismatches);
}
