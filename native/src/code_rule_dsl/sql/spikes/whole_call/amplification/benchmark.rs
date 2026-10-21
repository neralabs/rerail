//! Generated recursive rule over the configured real corpus. Keep its results separate from
//! the real policy catalog. Every completed plan must preserve all matching roots and captures.
use super::super::{graph::Graph, recursive_machine, run};
use super::{guards, relational};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{
        self,
        exec::{Captured, FileContext, SqlMatch},
        lower::ProgramBuilder,
        tests::{differential::tape, hard_cases::compile},
    },
    test_support::grammar_matchers,
};
use serde_json::{Value, json};
use std::path::Path;

#[test]
#[ignore = "set RERAIL_WHOLE_CALL_CONFIG and RERAIL_WHOLE_CALL_OUTPUT"]
fn recursive_amplification_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_WHOLE_CALL_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let output = std::env::var("RERAIL_WHOLE_CALL_OUTPUT").unwrap();
    let out = Path::new(&output);
    std::fs::create_dir_all(out).unwrap();
    let docs: Vec<_> = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let source = std::fs::read_to_string(
                Path::new(config["corpus"].as_str().unwrap()).join(p.as_str().unwrap()),
            )
            .unwrap();
            (
                tape(GrammarDialect::Python, &source),
                FileContext::default(),
            )
        })
        .collect();
    let policy = compile(
        "python",
        json!({"rule":{"kind":"function_definition","matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^(self|data)$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
    );
    let grammar = &grammar_matchers(&policy)[0];
    let runs = vec![sql::ClauseRun {
        statement: 0,
        files: (0..docs.len()).map(|file| (file, None)).collect(),
    }];
    let mut session = sql::exec::compile(&[grammar]).unwrap().session().unwrap();
    let started = crate::code_rule_dsl::sql::timing::Started::now();
    let expected = session.run(docs.clone(), &runs).unwrap();
    let legacy = started.elapsed();
    let mut builder = ProgramBuilder::spike(guards::apply);
    let statement = builder.clause_each(grammar, None).unwrap();
    let mut queries = Vec::new();
    for variant in [
        recursive_machine::LeafPlan::Baseline,
        recursive_machine::LeafPlan::Kinds,
        recursive_machine::LeafPlan::Capture,
        recursive_machine::LeafPlan::Partitioned,
    ] {
        queries.push((
            format!("machine-{variant:?}"),
            recursive_machine::compile_variant(&[&statement], &builder.terminals, variant).unwrap(),
        ));
    }
    for (name, group, binary) in [
        ("shared64", 64, false),
        ("binary", 0, true),
        ("shared64-binary", 64, true),
    ] {
        queries.push((
            format!("machine-{name}"),
            recursive_machine::compile_scoped(
                &[&statement],
                &builder.terminals,
                recursive_machine::LeafPlan::Partitioned,
                group,
                binary,
            )
            .unwrap(),
        ));
    }
    for variant in [
        relational::Shape::Closure,
        relational::Shape::Interval,
        relational::Shape::Probe,
        relational::Shape::Segments,
    ] {
        queries.push((
            format!("relational-{variant:?}"),
            relational::compile(&statement, &builder.terminals, variant).unwrap(),
        ));
    }
    let profiling = std::env::var_os("RERAIL_WHOLE_CALL_PROFILE").is_some();
    let reports = sql::exec::with_terminals(
        docs,
        builder.terminals,
        |connection, batch, frames| {
            recursive_machine::register_text(connection, batch).unwrap();
            let mut reports = Vec::new();
            for (name, query) in queries {
                if let Ok(selected) = std::env::var("RERAIL_RECURSIVE_CASES")
                    && !selected.split(',').any(|s| s == name)
                {
                    continue;
                }
                std::fs::write(out.join(format!("{name}.sql")), &query).unwrap();
                let mut observation = Value::Null;
                let report = match run::execute_observed(
                    connection,
                    frames,
                    &Graph::default(),
                    &[query],
                    profiling,
                    Some(&mut observation),
                ) {
                    Err(e) => {
                        json!({"case":name,"status":"rejected","error":e.to_string(),"parity":null,"metrics":observation})
                    }
                    Ok(got) => {
                        let mut actual = vec![vec![Vec::new(); batch.files.len()]];
                        let faults: Vec<_> =
                            got.hits.iter().filter(|h| h.3["fault"] != 0).collect();
                        for (_, file, root, value) in &got.hits {
                            if value["result"] == 1 && value["fault"] == 0 {
                                actual[0][*file].push(SqlMatch {
                                    root: *root,
                                    captures: vec![(
                                        statement.slots[0].name.clone(),
                                        Captured::Node(value["env"]["0"].as_u64().unwrap() as u32),
                                    )],
                                });
                            }
                        }
                        if faults.iter().any(|h| h.3["fault"] == 2) {
                            json!({"case":name,"status":"rejected","parity":null,"error":"root transition budget","faults":faults,"metrics":got.metrics})
                        } else {
                            json!({"case":name,"status":"completed","parity":faults.is_empty() && actual==expected,"matches":actual[0].iter().map(Vec::len).sum::<usize>(),"faults":faults,"metrics":got.metrics})
                        }
                    }
                };
                eprintln!(
                    "recursive amplification {name}: parity={} cpu={} vm={}",
                    report["parity"], report["metrics"]["cpuNs"], report["metrics"]["vmSteps"]
                );
                reports.push(report);
            }
            reports
        },
    );
    std::fs::write(out.join("results.json"),serde_json::to_vec_pretty(&json!({"scope":"one generated positive recursive rule over configured corpus files, not the policy catalog","files":config["files"],"legacyCpuNs":legacy.cpu,"legacyStatements":session.stats().statements,"alternatives":reports})).unwrap()).unwrap();
    assert!(
        reports.iter().all(|r| r["parity"] != false),
        "completed recursive plan changed roots or captures"
    );
}
