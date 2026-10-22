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
    let expected: Vec<Vec<u32>> = docs
        .iter()
        .map(|(doc, _)| {
            let tree = AstGrep::doc(doc.clone());
            let mut work = usize::MAX;
            (0..doc.transducer_len() as u32)
                .filter(|&id| {
                    grammar
                        .match_root(node_at(&tree, id), &mut work)
                        .unwrap()
                        .is_some()
                })
                .collect()
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
            baseline[0][file].iter().map(|m| m.root).collect::<Vec<_>>(),
            *expected
        );
        assert!(baseline[0][file].iter().all(|m| m.captures.is_empty()));
    }
    let started = Started::now();
    let compiled = super::compile(grammar).unwrap();
    let generation = started.elapsed();
    let started = Started::now();
    let (result, counters) = profiling::frame_counts(|| {
        sql::exec::with_terminals(docs, compiled.terminals, |db, _, _| {
            let started = Started::now();
            let mut statement = db.prepare(&compiled.sql).unwrap();
            let prepare = started.elapsed();
            let mut actual = vec![vec![]; sources.len()];
            {
                let mut rows = statement.query([]).unwrap();
                while let Some(row) = rows.next().unwrap() {
                    let file = row.get::<_, u32>(1).unwrap() as usize;
                    actual[file].push(row.get::<_, u32>(2).unwrap());
                    assert_eq!(row.get::<_, String>(3).unwrap(), "[]");
                }
            }
            let vm = statement.get_status(rusqlite::StatementStatus::VmStep);
            let automatic = statement.get_status(rusqlite::StatementStatus::AutoIndex);
            let scans = statement.get_status(rusqlite::StatementStatus::FullscanStep);
            let memory = statement.get_status(rusqlite::StatementStatus::MemUsed);
            let elapsed = started.elapsed();
            drop(statement);
            let explain = db.prepare(&format!("EXPLAIN QUERY PLAN {}", compiled.sql)).unwrap()
                .query_map([], |row| Ok(json!({"id":row.get::<_,i64>(0)?,"parent":row.get::<_,i64>(1)?,"detail":row.get::<_,String>(3)?})))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            (
                actual, vm, automatic, scans, memory, prepare, elapsed, explain,
            )
        })
    });
    let elapsed = started.elapsed().plus(generation);
    for (index, (actual, expected)) in result.0.iter().zip(&expected).enumerate() {
        assert_eq!(actual, expected, "{}", sources[index].0);
    }
    assert_eq!(counters[4], 0, "no frame publications");
    json!({"parity":true,"files":sources.len(),"matches":expected.iter().map(Vec::len).sum::<usize>(),
      "diagnosticCpuNs":elapsed.cpu,"generationCpuNs":generation.cpu,"prepareCpuNs":result.5.cpu,
      "prepareAndExecuteCpuNs":result.6.cpu,"explain":result.7,
      "timingScope":"Counters enabled. diagnosticCpuNs includes EXPLAIN; prepareAndExecuteCpuNs excludes EXPLAIN, connection setup, and generation. No speedup claim.",
      "baseline":{"diagnosticKernelCpuNs":baseline_time.cpu,"framePublications":baseline_counters[4],"frameAdvances":baseline_counters[2],"matchingStatements":baseline_session.stats().statements},
      "vmSteps":result.1,"automaticIndexRows":result.2,"fullscanSteps":result.3,"statementMemoryBytes":result.4,
      "framePublications":counters[4],"nodeAdvances":counters[11],"nodeFilters":counters[13],
      "regexRequests":counters[10],"matchingStatements":1,"sql":compiled.sql,"relations":compiled.relations})
}

fn policy(rule: Value, utils: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
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
            json!({"inside":{"stopBy":"end","kind":"function_definition"}}),
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
            json!({"r":{"any":[{"kind":"identifier"},{"has":{"stopBy":"neighbor","matches":"r"}}]}}),
            "ascending",
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
