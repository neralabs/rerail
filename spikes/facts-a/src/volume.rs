//! Phase 2 census, exported-relation volume: for the batch-6 rules whose repository clauses the
//! leaf executor runs, the witness rows of every input an exported relation scans, over the
//! corpus. A filter/project/distinct export holds at most its leaf input's rows, so these are
//! upper bounds for single-input exports (joins are reported with every leaf).
use crate::{eval, facts, leaf, repo};
use rerail_facts::ir::{FileSource, Rel, StageDef};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const RULES: &[&str] = &["serverplan-quality", "serverplan-state-tripwires", "serverplan-target-architecture"];

fn inputs_of(r: &Rel<FileSource>, stages: &[rerail_facts::ir::Stage<FileSource>], out: &mut BTreeSet<u32>) {
    rerail_facts::ir::map_rel(r.clone(), &mut |s| {
        match &s {
            FileSource::Input(i) => {
                out.insert(*i);
            }
            FileSource::Stage(j) => match &stages[*j as usize].def {
                StageDef::Query { rel } => inputs_of(rel, stages, out),
                StageDef::Reach { roots, edges, .. } => {
                    inputs_of(roots, stages, out);
                    if let Some(e) = edges {
                        inputs_of(e, stages, out);
                    }
                }
            },
            FileSource::Syntax(_) => {}
        }
        Ok::<_, ()>(s)
    })
    .unwrap();
}

pub fn run(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let root = crate::arg(args, "--root").unwrap_or(format!("{home}/rerail-bench/nera-data-layer-2b1504ad"));
    let rules_dir = crate::arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let threads: usize = crate::arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let out = std::path::PathBuf::from(crate::arg(args, "--out").expect("--out"));
    std::fs::create_dir_all(&out).unwrap();
    let root = Path::new(&root);
    let paths = repo::walk(root);
    let config = Map::new();
    let mut report = Vec::new();
    for rule in RULES {
        let doc: Json = serde_json::from_str(&std::fs::read_to_string(format!("{rules_dir}/data-layer-{rule}.json")).unwrap()).unwrap();
        let mut leaves = leaf::Leaves::new();
        let c = rerail_facts::compile::compile_report(&doc, &config, &mut leaves).expect("compile");
        let spike = eval::Rule::compile(&doc, &config, None, &facts::GRAMMAR).expect("spike compile");
        // Spike A numbers pattern inputs only (its regex clauses are not inputs).
        let mut spike_index: BTreeMap<u32, u32> = BTreeMap::new();
        for (k, def) in c.program.inputs.iter().enumerate() {
            if matches!(def.kind, rerail_facts::ir::InputKind::Pattern { .. }) {
                let n = spike_index.len() as u32;
                assert_eq!(spike.inputs[n as usize].fields, def.fields.iter().map(|f| f.0.clone()).collect::<Vec<_>>(), "{rule}: input {} fields differ", def.name);
                spike_index.insert(k as u32, n);
            }
        }
        assert_eq!(spike_index.len(), spike.inputs.len(), "{rule}: input numbering differs");
        // Inputs each export scans.
        let mut wanted: BTreeSet<u32> = BTreeSet::new();
        let mut per_export: Vec<(usize, BTreeSet<u32>)> = Vec::new();
        for (k, e) in c.program.exported.iter().enumerate() {
            let plan = &c.program.per_file[e.producer];
            let rerail_facts::ir::Sink::Export(list) = &plan.sink else { continue };
            let Some((_, def)) = list.iter().find(|(id, _)| *id == e.id) else { continue };
            let mut ins = BTreeSet::new();
            inputs_of(def, &plan.stages, &mut ins);
            wanted.extend(ins.iter().copied());
            per_export.push((k, ins));
        }
        let wanted: Vec<u32> = wanted.into_iter().collect();
        let next = AtomicUsize::new(0);
        drop(spike);
        let totals: Vec<BTreeMap<u32, u64>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    let doc = &doc;
                    let spike_index = &spike_index;
                    let config = &config;
                    let wanted = wanted.clone();
                    let next = &next;
                    let paths = &paths;
                    s.spawn(move || {
                        // Compiled per worker: Spike A's rules are not shared across threads.
                        let spike = eval::Rule::compile(doc, config, None, &facts::GRAMMAR).expect("spike compile");
                        let mut counts: BTreeMap<u32, u64> = BTreeMap::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(path) = paths.get(i) else { break };
                            if !spike.scope.matches(path) {
                                continue;
                            }
                            let Ok(source) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                            let source = Arc::new(source);
                            let Ok(facts::Built::Facts(f)) = facts::build(path, source.clone()) else { continue };
                            let ctx = eval::FileCtx { f: &f, path, source: &source };
                            for &w in &wanted {
                                let input = &spike.inputs[spike_index[&w] as usize];
                                if input.paths.as_ref().is_some_and(|p| !p.matches(path)) {
                                    continue;
                                }
                                let rows = input.rows(&ctx, &crate::rel::input_schema(&input.fields));
                                *counts.entry(w).or_default() += rows.table.rows.len() as u64;
                            }
                        }
                        counts
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut rows: BTreeMap<u32, u64> = BTreeMap::new();
        for t in totals {
            for (k, v) in t {
                *rows.entry(k).or_default() += v;
            }
        }
        for (k, ins) in per_export {
            let e = &c.program.exported[k];
            report.push(json!({
                "rule": rule, "export": e.name, "columns": e.schema.len(), "distinct": e.distinct, "ordered": e.ordered,
                "leaf_inputs": ins.iter().map(|i| json!({ "input": c.program.inputs[*i as usize].name, "rows": rows.get(i).copied().unwrap_or(0) })).collect::<Vec<_>>(),
            }));
        }
    }
    std::fs::write(out.join("export-volume.json"), serde_json::to_string_pretty(&json!({ "files": paths.len(), "exports": report })).unwrap()).unwrap();
    println!("{} exports measured over {} files", report.len(), paths.len());
}
