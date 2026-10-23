//! Hand-written complete B023 query. Test-only: no production selector or rule evaluator callback.
use super::{graph::Graph, run};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    language::GrammarDialect,
    matcher::node_at,
    sql::{
        self,
        exec::{Captured, FileContext, SqlMatch},
        profiling,
        tests::differential::tape,
        timing::Started,
    },
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::{
    AstGrep,
    matcher::{MatcherExt, Pattern},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn query() -> String {
    let tokens = regex::Regex::new(r"@([a-z_]+)@").unwrap();
    let template = std::fs::read_to_string(
        root().join("native/src/code_rule_dsl/sql/spikes/whole_call/b023.sql"),
    )
    .unwrap();
    tokens
        .replace_all(&template, |m: &regex::Captures<'_>| {
            GrammarDialect::Python.kinds().kind_id(&m[1]).to_string()
        })
        .into_owned()
}

fn policy() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn target_slot_policy(mutation: usize) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
    )
    .unwrap();
    document["clauses"][0]["forbid"]["ast"]["rule"] = json!({"matches":"target-slot"});
    document["clauses"][0]["violation"] =
        json!({"code":"slot","message":"slot","anchor":"witness.span"});
    let source = match mutation {
        0 => document.to_string(),
        1 => document
            .to_string()
            .replace("target-slot", "renamed-destructuring"),
        2 => document
            .to_string()
            .replace("\"assignment\"", "\"expression_statement\""),
        3 => document.to_string().replace("^for$", "^in$"),
        _ => unreachable!(),
    };
    CodeRuleDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap()
}

/// A capture-free demand view using the real policy's utility definitions. The compiler lowers
/// this ordinary DSL query; automatic extraction from the capture-bearing rule is a later step.
fn region_policy(mutation: usize) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let ast = &mut document["clauses"][0]["forbid"]["ast"];
    ast["utils"]["region-demand"] = json!({"all":[{"matches":"loop"},{"has":{"stopBy":{"matches":"function"},"matches":"function"}}]});
    ast["rule"] = json!({"all":[{"matches":"target-slot"},{"inside":{"stopBy":"end","matches":"region-demand"}}]});
    document["clauses"][0]["violation"] =
        json!({"code":"region-slot","message":"slot","anchor":"witness.span"});
    let source = match mutation {
        0 => document.to_string(),
        1 => document
            .to_string()
            .replace("target-slot", "renamed-target")
            .replace("region-demand", "renamed-region")
            .replace("\"loop\"", "\"renamed-loop\"")
            .replace("\"function\"", "\"renamed-function\""),
        2 => document
            .to_string()
            .replace("\"for_statement\"", "\"while_statement\""),
        3 => document
            .to_string()
            .replace("functools.reduce", "other.reduce"),
        _ => unreachable!(),
    };
    CodeRuleDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap()
}

#[test]
fn b023_compiler_region_fixtures_and_mutations() {
    let sources=["packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py","packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py"]
        .into_iter().map(|path|(path.into(),std::fs::read_to_string(root().join(path)).unwrap()))
        .chain(std::iter::once(("regions.py".into(),"outside = 0\nfor x in xs:\n    a, (b, *c) = vals\n    for y in ys:\n        f = lambda: x\n    def g():\n        for z in zs:\n            q = lambda: z\nfor ignored in xs:\n    functools.reduce(lambda: ignored, xs)\n".into())))
        .collect::<Vec<_>>();
    let mut counts = Vec::new();
    for mutation in 0..4 {
        let p = region_policy(mutation);
        let result = sql::relational::tests::measure(&grammar_matchers(&p)[0], &sources);
        assert!(
            !result["relations"]["regionRoots"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        counts.push(result["matches"].as_u64().unwrap());
        eprintln!(
            "region view mutation {mutation}: {} slots, {} VM",
            result["matches"], result["vmSteps"]
        );
    }
    assert_eq!(counts[0], counts[1]);
    assert_ne!(counts[0], counts[2]);
    assert_ne!(counts[0], counts[3]);
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_RELATIONAL_OUTPUT"]
fn b023_compiler_region_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let p = region_policy(0);
    let mut result = sql::relational::tests::measure(&grammar_matchers(&p)[0], &sources);
    result["inputs"]=json!(sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>());
    eprintln!(
        "generated region view: {} files, {} slots, {} VM",
        result["files"], result["matches"], result["vmSteps"]
    );
    std::fs::write(
        std::env::var("RERAIL_RELATIONAL_OUTPUT").unwrap(),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
fn b023_compiler_region_scaling() {
    let p = region_policy(0);
    let grammar = &grammar_matchers(&p)[0];
    let mut records = Vec::new();
    for size in [32, 128, 512] {
        for inside in [false, true] {
            let assignments = (0..size)
                .map(|i| format!("a{i}, (b{i}, *c{i}) = vals\n"))
                .collect::<String>();
            let source = if inside {
                format!(
                    "for x in xs:\n{}    f = lambda: x\n",
                    assignments
                        .lines()
                        .map(|s| format!("    {s}\n"))
                        .collect::<String>()
                )
            } else {
                format!("{assignments}for x in xs:\n    f = lambda: x\n")
            };
            let mut result =
                sql::relational::tests::measure(grammar, &[("scale.py".into(), source)]);
            result["size"] = json!(size);
            result["case"] = json!(if inside {
                "inside-region"
            } else {
                "outside-region"
            });
            for key in ["sql", "relations", "explain", "diagnosticRelationRows"] {
                result.as_object_mut().unwrap().remove(key);
            }
            records.push(result);
        }
    }
    assert_eq!(records[0]["matches"], records[4]["matches"]);
    assert_eq!(
        records[0]["vmSteps"], records[4]["vmSteps"],
        "unrelated assignments must not amplify SQL work"
    );
    assert!(
        records[5]["vmSteps"].as_u64().unwrap() <= 17 * records[1]["vmSteps"].as_u64().unwrap()
    );
    let mut calls = Vec::new();
    for size in [32, 128, 512] {
        let source = format!(
            "{}for x in xs:\n    f = lambda: x\n",
            "print(value)\n".repeat(size)
        );
        let mut result = sql::relational::tests::measure(grammar, &[("calls.py".into(), source)]);
        result["size"] = json!(size);
        result["case"] = json!("outside-calls");
        for key in ["sql", "relations", "explain", "diagnosticRelationRows"] {
            result.as_object_mut().unwrap().remove(key);
        }
        calls.push(result);
    }
    assert_eq!(
        calls[0]["vmSteps"], calls[2]["vmSteps"],
        "unrequested safe-slot candidates must not amplify SQL work"
    );
    records.extend(calls);
    if let Ok(path) = std::env::var("RERAIL_REGION_SCALE_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&records).unwrap()).unwrap();
    }
}

#[test]
fn b023_compiler_target_slot_fixtures_and_mutations() {
    let sources = [
        "packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py",
        "packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py",
    ].into_iter().map(|path|(path.into(),std::fs::read_to_string(root().join(path)).unwrap()))
        .chain(std::iter::once(("adversarial.py".into(),
            "a, (b, [c, *d]) = values\nx: int = 1\nx += 2\nfor e, (f, g) in xs:\n    pass\n[h for (i, j) in xs]\nprint(a, key=b)\n".into())))
        .collect::<Vec<_>>();
    let mut counts = Vec::new();
    for mutation in 0..4 {
        let policy = target_slot_policy(mutation);
        let result = sql::relational::tests::measure(&grammar_matchers(&policy)[0], &sources);
        counts.push(result["matches"].as_u64().unwrap());
        eprintln!(
            "target-slot mutation {mutation}: {} matches, {} VM",
            result["matches"], result["vmSteps"]
        );
    }
    assert_eq!(
        counts[0], counts[1],
        "utility names do not select a compiler template"
    );
    assert_ne!(
        counts[0], counts[2],
        "changed kinds affect the compiled rule"
    );
    assert_ne!(
        counts[0], counts[3],
        "changed regex affects the compiled rule"
    );
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_RELATIONAL_OUTPUT"]
fn b023_compiler_target_slot_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let policy = target_slot_policy(0);
    let mut result = sql::relational::tests::measure(&grammar_matchers(&policy)[0], &sources);
    result["inputs"]=json!(sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>());
    eprintln!(
        "compiled target-slot: {} files, {} matches, {} VM",
        result["files"], result["matches"], result["vmSteps"]
    );
    std::fs::write(
        std::env::var("RERAIL_RELATIONAL_OUTPUT").unwrap(),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
fn b023_compiler_target_slot_scaling() {
    let policy = target_slot_policy(0);
    let grammar = &grammar_matchers(&policy)[0];
    let mut records = Vec::new();
    for size in [32, 128, 512] {
        for repeated in [false, true] {
            let source = if repeated {
                (0..size)
                    .map(|i| format!("a{i}, (b{i}, *c{i}) = values\n"))
                    .collect::<String>()
            } else {
                format!("a, (b, *c) = values\n{}", "print(value)\n".repeat(size))
            };
            let mut result =
                sql::relational::tests::measure(grammar, &[("scale.py".into(), source)]);
            result["size"] = json!(size);
            result["case"] = json!(if repeated {
                "repeated-targets"
            } else {
                "irrelevant-calls"
            });
            result.as_object_mut().unwrap().remove("sql");
            result.as_object_mut().unwrap().remove("relations");
            result.as_object_mut().unwrap().remove("explain");
            records.push(result);
        }
    }
    assert_eq!(
        records[0]["vmSteps"], records[4]["vmSteps"],
        "irrelevant nodes must not amplify SQL work"
    );
    assert_eq!(
        records[1]["matches"].as_u64().unwrap() * 16,
        records[5]["matches"].as_u64().unwrap()
    );
    assert!(
        records[5]["vmSteps"].as_u64().unwrap() <= 17 * records[1]["vmSteps"].as_u64().unwrap()
    );
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_SCALE_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&records).unwrap()).unwrap();
    }
}

#[test]
fn b023_compiler_capture_demand_fixtures_and_mutations() {
    let sources = [
        "packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py",
        "packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py",
    ]
    .into_iter()
    .map(|p| (p.into(), std::fs::read_to_string(root().join(p)).unwrap()))
    .collect::<Vec<_>>();
    let source = std::fs::read_to_string(
        root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
    )
    .unwrap();
    let mut results = Vec::new();
    for mutation in 0..4 {
        let changed = match mutation {
            0 => source.clone(),
            1 => source
                .replace("target-slot", "renamed-target")
                .replace("loop-assigns-n", "renamed-producer")
                .replace("unbound-in-function", "renamed-consumer")
                .replace("$N", "$VALUE"),
            2 => source.replace("\"for_statement\"", "\"while_statement\""),
            3 => source.replace("functools.reduce", "other.reduce"),
            _ => unreachable!(),
        };
        let p = CodeRuleDslRuntime::compile(changed, "json".into(), CAPABILITIES.into()).unwrap();
        let result = sql::relational::witness_tests::measure(&grammar_matchers(&p)[0], &sources);
        let full = sql::relational::tests::measure(&grammar_matchers(&p)[0], &sources);
        eprintln!(
            "complete mutation {mutation}: {} matches, {} VM",
            full["matches"], full["vmSteps"]
        );
        assert!(
            result["profile"]["demandDecisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d.as_str().unwrap().contains("boundary is closed")),
            "target recursion must use structural demand"
        );
        eprintln!(
            "capture demand mutation {mutation}: {} rows, {} VM",
            result["rows"], result["metrics"]["vmSteps"]
        );
        results.push(result["output"].clone());
    }
    assert_eq!(results[0], results[1]);
    assert_ne!(results[0], results[2]);
    assert_ne!(results[0], results[3]);
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_CAPTURE_DEMAND_OUTPUT"]
fn b023_compiler_capture_demand_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let p = policy();
    let mut result = sql::relational::witness_tests::measure(&grammar_matchers(&p)[0], &sources);
    let reference = check(&sources, None, true);
    let projected: Vec<_> = result["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| json!([r[0], r[1], r[2], r[3]]))
        .collect();
    assert_eq!(
        json!(projected),
        reference["captureDemand"],
        "generated/reference exact capture demand"
    );
    result["referenceDemandParity"] = json!(true);
    result["referenceFullRuleMatches"] = reference["matches"].clone();
    result["inputs"]=json!(sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>());
    eprintln!(
        "capture demand corpus: {} files, {} rows, {} VM",
        result["files"], result["rows"], result["metrics"]["vmSteps"]
    );
    std::fs::write(
        std::env::var("RERAIL_CAPTURE_DEMAND_OUTPUT").unwrap(),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
fn b023_compiler_capture_demand_scaling() {
    let p = policy();
    let grammar = &grammar_matchers(&p)[0];
    let mut records = Vec::new();
    for outside in [true, false] {
        for size in [32, 128, 512] {
            let source = if outside {
                format!(
                    "{}for x in xs:\n    f = lambda: x\n",
                    (0..size)
                        .map(|i| format!("a{i} = value\n"))
                        .collect::<String>()
                )
            } else {
                format!(
                    "for x in xs:\n{}    f = lambda: x\n",
                    (0..size)
                        .map(|i| format!("    a{i} = value\n"))
                        .collect::<String>()
                )
            };
            let mut result =
                sql::relational::witness_tests::measure(grammar, &[("scale.py".into(), source)]);
            result["size"] = json!(size);
            result["case"] = json!(if outside {
                "outside-region"
            } else {
                "inside-region"
            });
            for k in ["sql", "explain", "output"] {
                result.as_object_mut().unwrap().remove(k);
            }
            records.push(result);
        }
    }
    assert_eq!(
        records[0]["metrics"]["vmSteps"],
        records[2]["metrics"]["vmSteps"]
    );
    assert!(
        records[5]["metrics"]["vmSteps"].as_u64().unwrap()
            <= 17 * records[3]["metrics"]["vmSteps"].as_u64().unwrap()
    );
    if let Ok(path) = std::env::var("RERAIL_CAPTURE_DEMAND_SCALING") {
        std::fs::write(path, serde_json::to_string_pretty(&records).unwrap()).unwrap();
    }
}

fn counters(c: [u64; 14]) -> Value {
    json!({"frameAdvances":c[2],"frameColumns":c[3],"framePublications":c[4],"frameIndexedRows":c[6],
      "regexNodeRequests":c[10],"nodeAdvances":c[11],"nodeColumns":c[12],"nodeFilters":c[13]})
}

fn check(sources: &[(String, String)], out: Option<&Path>, diagnostic: bool) -> Value {
    check_query(sources, out, diagnostic, false)
}

// Both query shapes use fresh connections, identical parsing, counters and trace instrumentation.
fn check_query(
    sources: &[(String, String)],
    out: Option<&Path>,
    diagnostic: bool,
    generated: bool,
) -> Value {
    let compiled = policy();
    let grammars = grammar_matchers(&compiled);
    let grammar = &grammars[0];
    assert!(
        !grammar.three_valued,
        "this query requires B023's total leaf contract"
    );
    let parse_start = Started::now();
    let docs: Vec<_> = sources
        .iter()
        .map(|(_, s)| (tape(GrammarDialect::Python, s), FileContext::default()))
        .collect();
    let parse = parse_start.elapsed();
    let native_start = Started::now();
    let native: Vec<Vec<SqlMatch>> = docs
        .iter()
        .map(|(doc, _)| {
            let tree = AstGrep::doc(doc.clone());
            let mut work = usize::MAX;
            (0..doc.transducer_len() as u32)
                .filter_map(|id| {
                    let n = node_at(&tree, id);
                    // A necessary top-level kind test, independent of the SQL result.
                    if n.kind().as_ref() != "identifier" {
                        return None;
                    }
                    grammar
                            .match_root(n, &mut work)
                            .expect("native work")
                            .map(|m| SqlMatch {
                                root: id,
                                captures: vec![(
                                    "N".into(),
                                    Captured::Node(
                                        m.get_env().get_match("N").unwrap().node_id() as u32
                                    ),
                                )],
                            })
                })
                .collect()
        })
        .collect();
    let native_time = native_start.elapsed();
    let baseline_start = Started::now();
    let mut session = sql::exec::compile(&[grammar]).unwrap().session().unwrap();
    let runs = [sql::ClauseRun {
        statement: 0,
        files: (0..docs.len()).map(|f| (f, None)).collect(),
    }];
    let (baseline, count) = if diagnostic {
        profiling::frame_counts(|| session.run(docs.clone(), &runs).unwrap())
    } else {
        (session.run(docs.clone(), &runs).unwrap(), [0; 14])
    };
    let baseline_time = baseline_start.elapsed();
    assert_eq!(baseline[0], native, "current SQL/native roots and captures");
    let mut report = json!({"files":sources.len(),"sourceBytes":sources.iter().map(|(_,s)|s.len()).sum::<usize>(),
      "inputs":sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>(),
      "matches":native.iter().map(Vec::len).sum::<usize>(),"parseCpuNs":parse.cpu,"nativeCpuNs":native_time.cpu,
      "baseline":{"coldKernelCpuNs":baseline_time.cpu,"matchingCpuNs":session.stats().time.cpu,
        "scheduledStatements":session.stats().statements,"counters":if diagnostic {counters(count)} else {Value::Null}}});
    let generation_start = Started::now();
    let compiled = generated.then(|| sql::relational::compile(grammar).unwrap());
    let (sql, terminals) = if let Some(c) = compiled {
        (c.sql, c.terminals)
    } else {
        (query(), vec![])
    };
    let generation = generation_start.elapsed();
    report["querySha256"] = json!(format!("{:x}", Sha256::digest(sql.as_bytes())));
    report["policySha256"] = json!(format!(
        "{:x}",
        Sha256::digest(
            std::fs::read(
                root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json")
            )
            .unwrap()
        )
    ));
    if let Some(out) = out {
        std::fs::create_dir_all(out).unwrap();
        std::fs::write(out.join("query.sql"), &sql).unwrap();
    }
    let direct_start = Started::now();
    sql::exec::with_terminals(docs, terminals, |connection, batch, frames| {
        sql::relational::leaves::register(connection, batch).unwrap();
        struct Leaves(std::panic::AssertUnwindSafe<Arc<sql::exec::Batch>>, Pattern);
        impl Leaves {
            fn text(&self, f: usize, n: u32) -> String {
                node_at(&self.0.files[f].root, n).text().into_owned()
            }
            fn reduce(&self, f: usize, n: u32) -> bool {
                self.1
                    .match_node(node_at(&self.0.files[f].root, n))
                    .is_some()
            }
        }
        let leaves = Arc::new(Leaves(
            std::panic::AssertUnwindSafe(Arc::clone(batch)),
            Pattern::new("functools.reduce", GrammarDialect::Python),
        ));
        let leaf_calls = Arc::new(AtomicUsize::new(0));
        for (name, is_pattern) in [("spike_node_text", false), ("spike_is_reduce", true)] {
            let l = Arc::clone(&leaves);
            let c = Arc::clone(&leaf_calls);
            connection
                .create_scalar_function(
                    name,
                    2,
                    rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                    move |ctx| {
                        c.fetch_add(1, Ordering::Relaxed);
                        let f = ctx.get::<i64>(0)? as usize;
                        let n = ctx.get::<u32>(1)?;
                        Ok(if is_pattern {
                            rusqlite::types::Value::Integer(i64::from(l.reduce(f, n)))
                        } else {
                            rusqlite::types::Value::Text(l.text(f, n))
                        })
                    },
                )
                .unwrap();
        }
        let execute = || {
            run::execute_program(
                connection,
                frames,
                &Graph::default(),
                std::slice::from_ref(&sql),
                diagnostic,
            )
        };
        let (result, count) = if diagnostic {
            profiling::frame_counts(execute)
        } else {
            (execute(), [0; 14])
        };
        let cold = direct_start.elapsed().plus(generation);
        let result = result.unwrap();
        let mut actual = vec![vec![]; sources.len()];
        for (_, file, id, value) in &result.hits {
            actual[*file].push(SqlMatch {
                root: *id,
                captures: vec![(
                    "N".into(),
                    Captured::Node(value[0].as_u64().unwrap() as u32),
                )],
            });
        }
        for (i, (got, want)) in actual.iter().zip(&native).enumerate() {
            assert_eq!(got, want, "direct/native: {}", sources[i].0);
        }
        assert_eq!(result.metrics["sqlExecutions"], 1);
        if diagnostic {
            assert_eq!(count[4], 0);
        }
        report["direct"] = json!({"coldKernelCpuNs":cold.cpu,"generationCpuNs":generation.cpu,"metrics":result.metrics,"counters":if diagnostic {counters(count)} else {Value::Null},"leafCalls":leaf_calls.load(Ordering::Relaxed)});
        if diagnostic {
            report["explain"] = super::profile::explain(connection, &sql);
        }
        // Matching performance uses repeated executions of this same prepared handle.
        // Installation CPU is retained separately and excluded from each sample.
        let mut prepared = run::PreparedQuery::new(connection, &sql).unwrap();
        let installation = prepared.preparation;
        let mut samples = Vec::new();
        for _ in 0..3 {
            let execution = prepared.execute().unwrap();
            assert_eq!(execution.hits, result.hits);
            assert_eq!(execution.metrics["prepareCpuNs"], 0);
            samples.push(execution.metrics);
        }
        report["prepared"] = json!({"preparationCount":1,"executionCount":3,"installationCpuNs":installation.cpu,"samples":samples});
        if diagnostic && !generated {
            let prefix = sql
                .rsplit_once("SELECT 0,file,id,json_array(capture)")
                .unwrap()
                .0;
            let names = [
                "loops",
                "safe_lambda",
                "functions",
                "function_climb",
                "function_loops",
                "relevant_loops",
                "regions",
                "target_slot",
                "targets",
                "assignment_climb",
                "loop_assigns",
                "demand",
                "store_slot",
                "stores",
                "parameter_walk",
                "parameters",
                "unbound",
                "bodies",
                "loads",
                "matches",
            ];
            let count_sql = format!(
                "{prefix}{}",
                names
                    .iter()
                    .map(|name| format!("SELECT '{name}',COUNT(*) FROM {name}"))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ")
            );
            let counts: Vec<(String, i64)> = connection
                .prepare(&count_sql)
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            report["diagnosticRelationRows"] = json!(counts);
            let mut demand_rows = connection
                .prepare(&format!(
                    "{prefix}SELECT file,fn,spike_node_text(file,atom),capture FROM demand"
                ))
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, u32>(3)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            demand_rows.sort();
            report["captureDemand"] = json!(demand_rows);
        }
    });
    report["parity"] = json!(true);
    report["generated"] = json!(generated);
    if let Some(out) = out {
        std::fs::write(
            out.join("results.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let outcomes:Vec<_>=native.iter().enumerate().flat_map(|(file,hits)|hits.iter().map(move|hit|json!({"file":file,"root":hit.root,"captures":format!("{:?}",hit.captures)}))).collect();
        std::fs::write(
            out.join("outcomes.json"),
            serde_json::to_vec_pretty(&outcomes).unwrap(),
        )
        .unwrap();
    }
    eprintln!(
        "B023 {} files {} matches: current {}ms direct {}ms / {} VM",
        sources.len(),
        report["matches"],
        report["baseline"]["coldKernelCpuNs"].as_u64().unwrap() / 1_000_000,
        report["direct"]["coldKernelCpuNs"].as_u64().unwrap() / 1_000_000,
        report["direct"]["metrics"]["vmSteps"]
    );
    report
}

#[test]
fn b023_direct_fixtures() {
    let paths = [
        "packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py",
        "packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py",
    ];
    let sources = paths
        .iter()
        .map(|p| {
            (
                p.to_string(),
                std::fs::read_to_string(root().join(p)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    check(&sources, None, true);
}

#[test]
fn b023_direct_adversarial() {
    let snippets = [
        "for x in xs:\n def f():\n  return x\n",
        "for x in xs:\n def f(x=x):\n  return x\n def g():\n  x=0\n  return x\n",
        "for x in xs:\n def f():\n  def nested():\n   x=0\n  return x\n",
        "for x in xs:\n a=lambda: x\n b=(lambda: x)()\n c=map(lambda: x, xs)\n d=sorted(xs,key=(lambda: x))\n e=functools.reduce(lambda: x,xs)\n",
        "for x in xs:\n a=functools . reduce((lambda: x),xs)\n b=other.reduce(lambda: x,xs)\n c=filter((lambda: x),xs)\n",
        "for (x,[y,*z]) in xs:\n def f():\n  return x,y,z\n",
        "while xs:\n x,y=xs\n x+=1\n def f():\n  return x,y\n",
        "a=[lambda: x for x in xs]\nb={x:lambda: x for x in xs}\nc=(lambda: x for x in xs)\n",
        "for x in xs:\n def f(*x):\n  return x\n def g(**x):\n  return x\n def h(x:int=0):\n  return x\n",
        "for x in xs:\n def f():\n  with resource() as (x,y):\n   pass\n  return x,y\n",
        "for x in xs:\n def f():\n  a=(x:=1)\n  return x\n",
        "for x in xs:\n def f():\n  return obj.x, call(x=x), x\n",
        "for x in xs:\n def f():\n  global x\n  del x\n  return x\n",
        "for x in xs:\n def f():\n  def g():\n   return x\n  return g\n",
        "for x in xs:\n for x in ys:\n  def f():\n   return x+x+x\n",
        "x=0\ndef f():\n return x\n",
        "for x in xs:\n def f():\n  return lambda: x\n",
        "for x in xs:\n f=lambda y=(lambda: x): y\n",
        "for x in xs:\n f=lambda: missing(x,x)\nfor y in ys:\n f=lambda: x\n",
        "for café in xs:\n f=lambda: café\n",
        "for x in xs:\n def f():\n  match obj:\n   case Point(x=x): pass\n  return x\n",
        "for x in xs:\n def f():\n  try: pass\n  except Error as x: pass\n  return x\n",
    ];
    let sources = snippets
        .iter()
        .enumerate()
        .map(|(i, s)| (format!("adversarial-{i}.py"), s.to_string()))
        .collect::<Vec<_>>();
    check(&sources, None, true);
    sql::relational::tests::measure(&grammar_matchers(&policy())[0], &sources);
}

#[test]
fn b023_direct_scaling() {
    let mut reports = Vec::new();
    for family in ["irrelevant_assignments", "repeated_names"] {
        let mut first_vm = 0;
        for size in [32, 128, 512] {
            let source = if family == "irrelevant_assignments" {
                format!(
                    "{}for x in xs:\n def f():\n  return x\n",
                    (0..size)
                        .map(|i| format!("noise_{i}={i}\n"))
                        .collect::<String>()
                )
            } else {
                format!(
                    "for x in xs:\n{} def f():\n{}",
                    " x=0\n".repeat(size),
                    "  consume(x)\n".repeat(size)
                )
            };
            let mut report = check(&[(format!("{family}-{size}.py"), source)], None, true);
            let vm = report["direct"]["metrics"]["vmSteps"].as_u64().unwrap();
            if size == 32 {
                first_vm = vm;
            }
            assert!(
                vm <= first_vm * (size as u64 / 32) * 2,
                "superlinear SQL work for {family}: {report}"
            );
            let expected = if family == "repeated_names" { size } else { 1 };
            assert_eq!(report["matches"], expected);
            report["family"] = json!(family);
            report["size"] = json!(size);
            reports.push(report);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_B023_SCALE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_B023_OUTPUT"]
fn b023_direct_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    check(
        &sources,
        Some(Path::new(&std::env::var("RERAIL_B023_OUTPUT").unwrap())),
        std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some(),
    );
}

#[test]
fn b023_compiler_complete_fixtures() {
    let paths = [
        "packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py",
        "packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py",
    ];
    let sources = paths
        .iter()
        .map(|p| {
            (
                p.to_string(),
                std::fs::read_to_string(root().join(p)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let result = sql::relational::tests::measure(&grammar_matchers(&policy())[0], &sources);
    eprintln!(
        "complete fixtures: {} matches, {} VM",
        result["matches"], result["vmSteps"]
    );
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_COMPLETE_OUTPUT"]
fn b023_compiler_complete_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let mut result = sql::relational::tests::measure(&grammar_matchers(&policy())[0], &sources);
    result["inputs"]=json!(sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>());
    let mut samples = vec![];
    for index in 0..3 {
        let (generated, reference) = if index % 2 == 0 {
            let generated = check_query(&sources, None, true, true);
            (generated, check_query(&sources, None, true, false))
        } else {
            let reference = check_query(&sources, None, true, false);
            (check_query(&sources, None, true, true), reference)
        };
        samples.push(json!({"generated":generated,"reference":reference}));
    }
    result["samples"] = json!(samples);
    eprintln!(
        "complete corpus: {} files, {} matches, {} VM, {} ms",
        result["files"],
        result["matches"],
        result["vmSteps"],
        result["prepareAndExecuteCpuNs"].as_u64().unwrap() / 1_000_000
    );
    std::fs::write(
        std::env::var("RERAIL_COMPLETE_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_CANDIDATE_PROFILE"]
fn b023_candidate_profile() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let name = p.as_str().unwrap();
            (
                name.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(name))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let mut report = check_query(&sources, None, true, true);
    report["sql"] = json!(
        sql::relational::compile(&grammar_matchers(&policy())[0])
            .unwrap()
            .sql
    );
    std::fs::write(
        std::env::var("RERAIL_CANDIDATE_PROFILE").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn b023_candidate_scaling() {
    let p = policy();
    let grammar = &grammar_matchers(&p)[0];
    let mut reports = Vec::new();
    for family in ["outside-functions", "unused-bindings"] {
        for size in [32, 128, 512] {
            let source = if family == "outside-functions" {
                format!(
                    "{}for x in xs:\n def f():\n  return x\n",
                    (0..size)
                        .map(|i| format!("def noise_{i}():\n return {i}\n"))
                        .collect::<String>()
                )
            } else {
                format!(
                    "for x in xs:\n{} def f():\n  return x\n",
                    (0..size)
                        .map(|i| format!(" unused_{i}={i}\n"))
                        .collect::<String>()
                )
            };
            let mut report = sql::relational::tests::measure(
                grammar,
                &[(format!("{family}-{size}.py"), source)],
            );
            assert_eq!(report["matches"], 1);
            report["family"] = json!(family);
            report["size"] = json!(size);
            reports.push(report);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_CANDIDATE_SCALING") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
    // Persist all observations before reporting the initially failing budgets.
    let outside: Vec<_> = reports
        .iter()
        .filter(|r| r["family"] == "outside-functions")
        .collect();
    assert_eq!(
        outside[0]["vmSteps"], outside[2]["vmSteps"],
        "unrelated functions must not add query work"
    );
    for report in reports.iter().filter(|r| r["family"] == "unused-bindings") {
        let demand = report["diagnosticRelationRows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["relation"] == "capture_demand")
            .unwrap();
        assert_eq!(
            demand["rows"], 1,
            "unused names must not enter capture demand"
        );
    }
}

#[test]
fn b023_candidate_preserves_stops_and_value_witnesses() {
    let sources = [
        "for x in xs:\n x=1\n x=2\n unused=3\n def f():\n  return x+x\n",
        "for x in xs:\n for y in ys:\n  x=2\n  def f():\n   return x\n",
        "for x in xs:\n def stopped():\n  def nested():\n   return x\n",
        "for x in xs:\n def stopped():\n  for y in ys:\n   def nested():\n    return x,y\n",
        "for x in xs:\n f=lambda: x\n g=map(lambda: x,xs)\n h=sorted(xs,key=lambda: x)\n",
        "for x in xs:\n f=lambda x=x: x\n def g():\n  return obj.x\n",
        "for x in xs:\n def f():\n  consume(x)\n  x=1\n def g():\n  def h():\n   x=2\n  return x\n",
        "for café in xs:\n unused=1\n f=lambda: café\n",
        "for x in xs:\n def f():\n  return y\nfor y in ys:\n def g():\n  return x\n",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, s)| (format!("candidate-{i}.py"), s.to_owned()))
    .collect::<Vec<_>>();
    let result = sql::relational::tests::measure(&grammar_matchers(&policy())[0], &sources);
    assert_eq!(result["parity"], true);
}

#[test]
fn b023_compiler_complete_scaling() {
    let p = policy();
    let grammar = &grammar_matchers(&p)[0];
    let mut records = vec![];
    for family in ["outside-assignments", "repeated-assignment-loads"] {
        let mut first = 0;
        for size in [32, 128, 512] {
            let source = if family == "outside-assignments" {
                format!(
                    "{}for x in xs:\n def f():\n  return x\n",
                    (0..size)
                        .map(|i| format!("noise_{i}={i}\n"))
                        .collect::<String>()
                )
            } else {
                format!(
                    "for x in xs:\n{} def f():\n{}",
                    " x=0\n".repeat(size),
                    "  consume(x)\n".repeat(size)
                )
            };
            let mut record = sql::relational::tests::measure(
                grammar,
                &[(format!("{family}-{size}.py"), source)],
            );
            let vm = record["vmSteps"].as_u64().unwrap();
            if size == 32 {
                first = vm;
            }
            if family == "outside-assignments" {
                assert_eq!(vm, first, "unrelated assignments amplify SQL work");
            } else {
                assert!(
                    vm <= first * (size as u64 / 32) * 2,
                    "superlinear combined assignment/load work"
                );
            }
            assert_eq!(
                record["matches"],
                if family == "outside-assignments" {
                    1
                } else {
                    size
                }
            );
            for key in ["sql", "relations", "explain", "diagnosticRelationRows"] {
                record.as_object_mut().unwrap().remove(key);
            }
            record["family"] = json!(family);
            record["size"] = json!(size);
            eprintln!("complete scaling {family} {size}: {vm} VM");
            records.push(record);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_COMPLETE_SCALING") {
        std::fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
}

#[test]
fn b023_compiler_complete_rejects_zero_distance_recursion() {
    let mut document: Value = serde_json::from_str(
        &std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
    )
    .unwrap();
    document["clauses"][0]["forbid"]["ast"]["utils"]["param-n"] =
        json!({"any":[{"matches":"param-n"},{"kind":"identifier","pattern":"$N"}]});
    let error =
        match CodeRuleDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
        {
            Err(error) => error.to_string(),
            Ok(policy) => match sql::relational::compile(&grammar_matchers(&policy)[0]) {
                Err(error) => error,
                Ok(_) => panic!("accepted non-descending recursion"),
            },
        };
    assert!(
        error.contains("cyclic dependency") || error.contains("strictly down"),
        "{error}"
    );
}

#[test]
fn b023_compiler_reference_shape_preserves_direct_body_priority() {
    let template: Value = serde_json::from_str(
        &std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let sources = vec![
        (
            "nested.py".into(),
            "for x in xs:\n f=lambda: x\n def g():\n  for x in ys:\n   h=lambda: x\n  return x\n"
                .into(),
        ),
        (
            "shadow.py".into(),
            "for x in xs:\n f=lambda x=x: x\n g=lambda: consume(x)\n def h():\n  x=0\n  return x\n"
                .into(),
        ),
        ("empty.py".into(), String::new()),
    ];
    for reverse in [false, true] {
        let mut document = template.clone();
        if reverse {
            document["clauses"][0]["forbid"]["ast"]["rule"]["all"][2]["any"]
                .as_array_mut()
                .unwrap()
                .reverse();
        }
        let policy =
            CodeRuleDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
                .unwrap();
        let measured = sql::relational::tests::measure(&grammar_matchers(&policy)[0], &sources);
        assert_eq!(measured["parity"], true);
        assert_eq!(
            measured["sql"].as_str().unwrap().contains("n.id>=b.id"),
            !reverse
        );
    }
}
