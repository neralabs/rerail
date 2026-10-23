use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{GrammarMatcher, node_at},
    sql::{self, exec::FileContext, profiling, tests::differential::tape, timing::Started},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

pub(in crate::code_rule_dsl::sql) fn measure(
    grammar: &GrammarMatcher,
    sources: &[(String, String)],
) -> Value {
    let docs: Vec<_> = sources
        .iter()
        .map(|(_, s)| (tape(grammar.language, s), FileContext::default()))
        .collect();
    let capture_names: Vec<_> = crate::code_rule_dsl::matcher::CaptureAnalysis::default()
        .binds(&grammar.matcher)
        .union(&crate::code_rule_dsl::matcher::CaptureAnalysis::default().counts(&grammar.matcher))
        .cloned()
        .collect();
    let expected: Vec<Vec<(u32, Vec<Value>)>> = docs
        .iter()
        .map(|(doc, _)| {
            let tree = AstGrep::doc(doc.clone());
            let mut work = usize::MAX;
            crate::code_rule_dsl::matcher::with_file_memo(&tree, || {
                (0..doc.transducer_len() as u32)
                    .filter_map(|id| {
                        grammar
                            .match_root(node_at(&tree, id), &mut work)
                            .unwrap()
                            .map(|m| {
                                (
                                    id,
                                    capture_names
                                        .iter()
                                        .map(|name| {
                                            m.get_env()
                                                .get_match(name)
                                                .map(|n| json!(n.node_id() as u32))
                                                .or_else(|| {
                                                    m.get_env().get_transformed(name).map(|s| {
                                                        json!(
                                                            String::from_utf8_lossy(s).into_owned()
                                                        )
                                                    })
                                                })
                                                .unwrap_or(Value::Null)
                                        })
                                        .collect(),
                                )
                            })
                    })
                    .collect()
            })
        })
        .collect();
    let baseline_start = Started::now();
    let mut baseline_session = sql::compile(&[grammar]).unwrap().session().unwrap();
    let (baseline, baseline_counters) = profiling::frame_counts(|| {
        baseline_session
            .run(
                docs.clone(),
                &[sql::ClauseRun {
                    statement: 0,
                    files: (0..docs.len()).map(|file| (file, None)).collect(),
                }],
            )
            .unwrap()
    });
    let baseline_time = baseline_start.elapsed();
    for (file, expected) in expected.iter().enumerate() {
        assert_eq!(
            baseline[0][file]
                .iter()
                .map(|m| (
                    m.root,
                    capture_names
                        .iter()
                        .map(|name| {
                            match m
                                .captures
                                .iter()
                                .find(|(key, _)| key == name)
                                .map(|(_, v)| v)
                            {
                                Some(sql::exec::Captured::Node(id)) => json!(id),
                                Some(sql::exec::Captured::Count(count)) => json!(count.to_string()),
                                None => Value::Null,
                                _ => panic!("unsupported capture output"),
                            }
                        })
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>(),
            *expected
        );
    }
    let started = Started::now();
    let compiled = super::compile(grammar).unwrap();
    let generation = started.elapsed();
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_QUERY_DUMP") {
        std::fs::write(path, &compiled.sql).unwrap();
    }
    let started = Started::now();
    let kernel_started = Started::now();
    let (result, _) = profiling::frame_counts(|| {
        sql::exec::with_terminals(docs, compiled.terminals, |db, batch, _| {
            let patterns = super::leaves::register(db, batch).unwrap();
            let started = Started::now();
            let counter_start = profiling::frame_snapshot();
            let value_start = super::leaves::VALUE_REQUESTS.get();
            let row_start = super::row::ROWS.get();
            let cell_start = super::row::CELLS.get();
            let equality_start = super::leaves::EQUALITY_REQUESTS.get();
            let mut statement = db.prepare(&compiled.sql).unwrap();
            let prepare = started.elapsed();
            let trace = ExecutionTrace::start(db);
            let mut actual = vec![vec![]; sources.len()];
            {
                let mut rows = statement.query([]).unwrap();
                while let Some(row) = rows.next().unwrap() {
                    let file = row.get::<_, u32>(1).unwrap() as usize;
                    actual[file].push((
                        row.get::<_, u32>(2).unwrap(),
                        serde_json::from_str::<Vec<Value>>(&row.get::<_, String>(3).unwrap())
                            .unwrap(),
                    ));
                }
            }
            let executions = trace.count();
            drop(trace);
            assert_eq!(
                executions, 1,
                "actual matching SQL executions, including callbacks"
            );
            let vm = statement.get_status(rusqlite::StatementStatus::VmStep);
            let automatic = statement.get_status(rusqlite::StatementStatus::AutoIndex);
            let scans = statement.get_status(rusqlite::StatementStatus::FullscanStep);
            let memory = statement.get_status(rusqlite::StatementStatus::MemUsed);
            let elapsed = started.elapsed();
            let cold = kernel_started.elapsed().plus(generation);
            let value_calls = super::leaves::VALUE_REQUESTS.get() - value_start;
            let typed_rows = super::row::ROWS.get() - row_start;
            let typed_cells = super::row::CELLS.get() - cell_start;
            let equality_calls = super::leaves::EQUALITY_REQUESTS.get() - equality_start;
            let pattern_calls = patterns.load(std::sync::atomic::Ordering::Relaxed);
            let counter_end = profiling::frame_snapshot();
            let counters: [u64; 14] = std::array::from_fn(|i| counter_end[i] - counter_start[i]);
            drop(statement);
            let explain = db.prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql)).unwrap()
                .query_map([], |row| Ok(json!({"id":row.get::<_,i64>(0)?,"parent":row.get::<_,i64>(1)?,"detail":row.get::<_,String>(3)?})))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            let mut cardinalities = Vec::new();
            if let Some(output) = compiled.relations["output"].as_str() {
                let prefix = compiled
                    .sql
                    .rsplit_once("SELECT 0,file,node,json_array(witness)")
                    .unwrap()
                    .0;
                let names: Vec<_> = compiled
                    .sql
                    .lines()
                    .filter_map(|line| line.split_once("(file,").map(|(name, _)| name))
                    .filter(|name| name.starts_with("answer"))
                    .chain(std::iter::once("capture_demand"))
                    .collect();
                for name in &names {
                    let rows: i64 = db
                        .query_row(&format!("{prefix}SELECT COUNT(*) FROM {name}"), [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    cardinalities.push(json!({"relation":name,"rows":rows}));
                }
                assert!(names.contains(&output));
            }
            if let Some(roots) = compiled.relations["regionRoots"]
                .as_array()
                .filter(|r| !r.is_empty())
            {
                let prefix = compiled
                    .sql
                    .rsplit_once("SELECT 0,file,node,json_array() FROM r0")
                    .unwrap()
                    .0;
                let mut names: Vec<_> = compiled.relations["relations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| format!("r{}", r["id"].as_u64().unwrap()))
                    .collect();
                for root in roots {
                    names.push(format!("regions{}", root.as_u64().unwrap()));
                }
                for relation in compiled.relations["relations"].as_array().unwrap() {
                    if !relation["keyDemand"].is_null() {
                        names.push(format!("needed{}", relation["id"].as_u64().unwrap()));
                    }
                }
                let counts = names
                    .iter()
                    .map(|n| format!("SELECT '{n}',COUNT(*) FROM {n}"))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ");
                cardinalities = db
                    .prepare(&format!("{prefix}{counts}"))
                    .unwrap()
                    .query_map([], |row| {
                        Ok(json!({"relation":row.get::<_,String>(0)?,"rows":row.get::<_,i64>(1)?}))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
            }
            if let Some(components) = compiled.relations["components"].as_array() {
                let prefix = compiled
                    .sql
                    .rsplit_once("SELECT 0,file,node,json_array() FROM r0")
                    .unwrap()
                    .0;
                for component in components {
                    let name = format!("component{}", component["id"].as_u64().unwrap());
                    let rows: i64 = db
                        .query_row(&format!("{prefix}SELECT COUNT(*) FROM {name}"), [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    cardinalities.push(json!({"relation":name,"rows":rows,"diagnostic":true}));
                }
            }
            let mut outcomes = Vec::new();
            if let Some(output) = compiled.relations["outcomeOutput"].as_str() {
                let prefix = compiled.sql.split_once("-- final matches").unwrap().0;
                let demand = compiled.relations["outcomeDemand"].as_str().unwrap();
                outcomes = db.prepare(&format!("{prefix}SELECT d.file,d.node,o.truth FROM {demand} d JOIN {output} o ON o.k=d.k WHERE o.truth<>0 ORDER BY d.file,d.node"))
                    .unwrap().query_map([], |r| Ok((r.get::<_,u32>(0)? as usize,r.get::<_,u32>(1)?,r.get::<_,u8>(2)?)))
                    .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                for relation in compiled.relations["relations"].as_array().unwrap() {
                    let name = relation["relation"].as_str().unwrap();
                    let rows: i64 = db
                        .query_row(&format!("{prefix}SELECT COUNT(*) FROM {name}"), [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    cardinalities.push(json!({"relation":name,"rows":rows,"diagnostic":true}));
                }
            }
            (
                actual,
                vm,
                automatic,
                scans,
                memory,
                prepare,
                elapsed,
                explain,
                cardinalities,
                pattern_calls,
                counters,
                cold,
                executions,
                outcomes,
                equality_calls,
                value_calls,
                typed_rows,
                typed_cells,
            )
        })
    });
    let elapsed = started.elapsed().plus(generation);
    let counters = result.10;
    for (index, (actual, expected)) in result.0.iter().zip(&expected).enumerate() {
        assert_eq!(actual, expected, "{}", sources[index].0);
    }
    assert_eq!(counters[4], 0, "no frame publications");
    if compiled.relations["outcomeOutput"].is_string() {
        let mut native = Vec::new();
        for (file, (_, source)) in sources.iter().enumerate() {
            let doc = tape(grammar.language, source);
            let len = doc.transducer_len();
            let tree = AstGrep::doc(doc);
            crate::code_rule_dsl::matcher::with_file_memo(&tree, || {
                for id in 0..len as u32 {
                    if !grammar.applies {
                        continue;
                    }
                    let truth = crate::code_rule_dsl::matcher::leaf_verdict(
                        &grammar.matcher,
                        node_at(&tree, id),
                        &Default::default(),
                    )
                    .unwrap();
                    if truth != Some(false) {
                        native.push((file, id, if truth == Some(true) { 1u8 } else { 2u8 }));
                    }
                }
            });
        }
        assert_eq!(result.13, native, "exact true AND unknown root outcomes");
    }

    json!({"parity":true,"outcomes":result.13,"output":expected,"files":sources.len(),"matches":expected.iter().map(Vec::len).sum::<usize>(),
      "coldKernelCpuNs":result.11.cpu,"diagnosticCpuNs":elapsed.cpu,"generationCpuNs":generation.cpu,"prepareCpuNs":result.5.cpu,
      "prepareAndExecuteCpuNs":result.6.cpu,"explain":result.7,
      "diagnosticRelationRows":result.8,
      "typedReturnRows":result.16,"typedReturnCells":result.17,
      "patternRequests":result.9,"captureEqualityRequests":result.14,"valueRequests":result.15,
      "timingScope":"Counters enabled. diagnosticCpuNs includes EXPLAIN; prepareAndExecuteCpuNs excludes EXPLAIN, connection setup, and generation. No speedup claim.",
      "baseline":{"diagnosticKernelCpuNs":baseline_time.cpu,"framePublications":baseline_counters[4],"frameAdvances":baseline_counters[2],"matchingStatements":baseline_session.stats().statements},
      "vmSteps":result.1,"automaticIndexRows":result.2,"fullscanSteps":result.3,"statementMemoryBytes":result.4,
      "framePublications":counters[4],"nodeAdvances":counters[11],"nodeFilters":counters[13],
      "regexRequests":counters[10],"matchingStatements":result.12,"sql":compiled.sql,"relations":compiled.relations})
}

pub(super) fn policy(rule: Value, utils: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"relational-subset","revision":1,
      "mode":"steer","scope":{"include":["**/*"],"exclude":[]},"clauses":[{"id":"rule",
        "forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},
        "violation":{"code":"subset","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

#[test]
fn relational_neighbor_sets_preserve_native_semantics() {
    let sources = vec![
        (
            "a.py".into(),
            "a, (b, *c) = value\nfor d, e in xs:\n    f += d\n# comment\nx = y + z\n".into(),
        ),
        ("b.py".into(), "x: int = 3\nfoo(a=1)\n".into()),
    ];
    let rules = vec![
        json!({"kind":"identifier"}),
        json!({"any":[{"kind":"identifier"},{"regex":"^[a-z]$"}]}),
        json!({"all":[{"kind":"identifier"},{"not":{"regex":"^a$"}}]}),
        json!({"inside":{"stopBy":"neighbor","kind":"assignment"}}),
        json!({"has":{"stopBy":"neighbor","kind":"identifier"}}),
        json!({"follows":{"stopBy":"neighbor","regex":"^for$"}}),
        json!({"precedes":{"stopBy":"neighbor","regex":"^=$"}}),
        json!({"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}}),
        json!({"not":{"any":[{"inside":{"stopBy":"neighbor","kind":"assignment"}},{"kind":"identifier"}]}}),
        json!({"has":{"stopBy":"neighbor","any":[{"kind":"identifier"},{"kind":"integer"}]}}),
    ];
    for rule in rules {
        let p = policy(rule, json!({}));
        measure(&grammar_matchers(&p)[0], &sources);
    }
}

#[test]
fn relational_positive_parent_recursion_and_shared_utilities() {
    let p = policy(
        json!({"all":[{"matches":"descendant"},{"not":{"matches":"leaf"}}]}),
        json!({
          "leaf":{"kind":"identifier"},
          "descendant":{"any":[{"kind":"assignment"},{"inside":{"stopBy":"neighbor","matches":"descendant"}}]}
        }),
    );
    let result = measure(
        &grammar_matchers(&p)[0],
        &[("a.py".into(), "a, (b, c) = (1, (2, 3))\nprint(a)\n".into())],
    );
    assert!(
        result["relations"]["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["recursive"] == true)
    );
    assert!(result["matches"].as_u64().unwrap() > 0);
}

#[test]
fn relational_unsupported_semantics_fail_closed() {
    let cases = vec![
        (json!({"pattern":"$X"}), json!({}), "capture-bearing"),
        (
            json!({"follows":{"stopBy":"end","kind":"function_definition"}}),
            json!({}),
            "outside",
        ),
        (
            json!({"matches":"r"}),
            json!({"r":{"any":[{"kind":"identifier"},{"inside":{"stopBy":"neighbor","not":{"matches":"r"}}}]}}),
            "recursive negation",
        ),
        (
            json!({"matches":"r"}),
            json!({"r":{"any":[{"kind":"identifier"},{"all":[{"inside":{"stopBy":"neighbor","matches":"r"}},{"inside":{"stopBy":"neighbor","matches":"r"}}]}]}}),
            "nonlinear",
        ),
    ];
    for (rule, utils, error) in cases {
        let p = policy(rule, utils);
        let Err(actual) = super::compile(&grammar_matchers(&p)[0]) else {
            panic!("accepted {error}");
        };
        assert!(actual.contains(error), "{actual}");
    }
}

#[test]
fn relational_expansion_limits_fail_before_execution() {
    let all: Vec<_> = (0..8)
        .map(|i| json!({"any":[{"regex":format!("a{i}")},{"regex":format!("b{i}")}]}))
        .collect();
    let p = policy(json!({"all":all}), json!({}));
    let Err(error) = super::compile(&grammar_matchers(&p)[0]) else {
        panic!("unbounded join-arm expansion");
    };
    assert!(error.contains("128"), "{error}");
    let mut rule = json!({"kind":"identifier"});
    for _ in 0..32 {
        rule = json!({"inside":{"stopBy":"neighbor","all":[rule]}});
    }
    let p = policy(rule, json!({}));
    let Err(error) = super::compile(&grammar_matchers(&p)[0]) else {
        panic!("unbounded join width");
    };
    assert!(error.contains("32"), "{error}");
}

#[test]
fn relational_regions_preserve_external_seeds_and_other_consumers() {
    let sources = vec![(
        "regions.py".into(),
        "a, (b, c) = values\noutside = other\n".into(),
    )];
    let utility = json!({"any":[
        {"inside":{"stopBy":"neighbor","kind":"assignment"}},
        {"inside":{"stopBy":"neighbor","any":[{"kind":"pattern_list"},{"kind":"tuple_pattern"}],"matches":"slots"}}
    ]});
    for (region, closed) in [("assignment", true), ("tuple_pattern", false)] {
        let selected =
            json!({"all":[{"matches":"slots"},{"inside":{"stopBy":"end","matches":"region"}}]});
        let utils = json!({"slots":utility,"region":{"kind":region}});
        let p = policy(selected.clone(), utils.clone());
        let result = measure(&grammar_matchers(&p)[0], &sources);
        assert!(result["matches"].as_u64().unwrap() > 0);
        let decisions = result["relations"]["demandDecisions"].as_array().unwrap();
        assert!(
            decisions
                .iter()
                .any(|d| d.as_str().unwrap().contains(if closed {
                    "boundary is closed"
                } else {
                    "can cross"
                })),
            "{decisions:?}"
        );
        let p = policy(json!({"any":[selected,{"matches":"slots"}]}), utils);
        measure(&grammar_matchers(&p)[0], &sources);
    }
}

#[test]
fn relational_ancestor_closure_stops_and_pattern_leaves() {
    let sources=vec![("stops.py".into(),"for x in xs:\n    def f():\n        def g():\n            foo.bar\n        return 0\nfoo.baz\n".into())];
    for rule in [
        json!({"has":{"stopBy":"end","pattern":"foo.bar"}}),
        json!({"has":{"stopBy":{"kind":"function_definition"},"kind":"function_definition"}}),
        json!({"inside":{"stopBy":"end","kind":"for_statement"}}),
    ] {
        let p = policy(rule, json!({}));
        measure(&grammar_matchers(&p)[0], &sources);
    }
}

#[test]
fn relational_key_demand_retains_recursive_dependencies_and_shared_answers() {
    let utils = json!({"safe":{"any":[
        {"inside":{"stopBy":"neighbor","kind":"return_statement"}},
        {"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"safe"}}
    ]}});
    let restricted = json!({"all":[{"kind":"lambda"},{"not":{"matches":"safe"}}]});
    let sources = vec![(
        "keys.py".into(),
        "def f():\n    return (((lambda: 1)))\ng = lambda: 2\n".into(),
    )];
    let p = policy(restricted.clone(), utils.clone());
    let result = measure(&grammar_matchers(&p)[0], &sources);
    assert_eq!(result["matches"], 1);
    assert!(
        result["relations"]["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| !r["keyDemand"].is_null())
    );
    let p = policy(json!({"any":[restricted,{"matches":"safe"}]}), utils);
    measure(&grammar_matchers(&p)[0], &sources);
}

/// Count the matching interval only. Keep the counter at a stable address until callbacks stop.
pub(super) struct ExecutionTrace<'a> {
    db: &'a rusqlite::Connection,
    count: Box<std::sync::atomic::AtomicUsize>,
}
impl<'a> ExecutionTrace<'a> {
    pub(super) fn start(db: &'a rusqlite::Connection) -> Self {
        unsafe extern "C" fn trace(
            mask: u32,
            context: *mut std::ffi::c_void,
            _: *mut std::ffi::c_void,
            _: *mut std::ffi::c_void,
        ) -> i32 {
            if mask == rusqlite::ffi::SQLITE_TRACE_STMT {
                // SAFETY: ExecutionTrace owns this allocation and unregisters before dropping it.
                let count = unsafe { &*context.cast::<std::sync::atomic::AtomicUsize>() };
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            0
        }
        let count = Box::new(std::sync::atomic::AtomicUsize::new(0));
        // SAFETY: the connection and stable counter outlive the installed callback.
        let result = unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                db.handle(),
                rusqlite::ffi::SQLITE_TRACE_STMT,
                Some(trace),
                (&*count as *const std::sync::atomic::AtomicUsize)
                    .cast_mut()
                    .cast(),
            )
        };
        assert_eq!(result, rusqlite::ffi::SQLITE_OK);
        Self { db, count }
    }
    pub(super) fn count(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Drop for ExecutionTrace<'_> {
    fn drop(&mut self) {
        // SAFETY: unregister before the counter is freed, including during unwinding.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(self.db.handle(), 0, None, std::ptr::null_mut());
        }
    }
}
