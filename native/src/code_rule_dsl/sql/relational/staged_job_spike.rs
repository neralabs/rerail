//! One physical table family can serve many rule producers without one CTAS per rule.
//! Native Kind/Regex matchers remain the oracle; callbacks only count atomic node reads.
use super::{spike_stages, tests::ExecutionTrace};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    sql::{exec, terminals::Terminal, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use rusqlite::StatementStatus;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[test]
fn structural_spike_staged_families_bound_cuts_across_rules_and_files() {
    let policies = [("identifier", "^(alpha|beta)$"), ("integer", "^[135]$")].map(|(kind, regex)| {
        CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"stage-family","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"python","rule":{"kind":kind,"regex":regex}}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    });
    let grammars = policies
        .iter()
        .map(|p| grammar_matchers(p).first().unwrap())
        .collect::<Vec<_>>();
    let language = grammars[0].language;
    let kinds = [
        language.kinds().kind_id("identifier"),
        language.kinds().kind_id("integer"),
    ];
    let source = "alpha\nbeta\nother\n1\n2\n3\n";
    let mut samples = vec![];
    for files in [1, 16] {
        for rules in [2, 32, 146] {
            let mut native = vec![];
            for clause in 0..rules {
                for file in 0..files {
                    let tree = AstGrep::doc(tape(language, source));
                    for node in tree.root().dfs() {
                        let mut work = 10000;
                        if grammars[clause % 2]
                            .match_root(node.clone(), &mut work)
                            .unwrap()
                            .is_some()
                        {
                            native.push([clause as i64, file as i64, node.node_id() as i64]);
                        }
                        assert_eq!(work, 10000, "this control admits only pure zero-cost atoms");
                    }
                }
            }
            native.sort();
            let mut metrics = vec![];
            for staged in [false, true] {
                let visits = Arc::new(AtomicU64::new(0));
                let terminals = ["^(alpha|beta)$", "^[135]$"]
                    .map(|r| Terminal::Regex(regex::Regex::new(r).unwrap()))
                    .into();
                let docs = (0..files)
                    .map(|_| (tape(language, source), Default::default()))
                    .collect();
                let sample = exec::with_terminals(docs, terminals, |db, _, _| {
                    let observed = visits.clone();
                    db.create_scalar_function(
                        "staged_producer_visit",
                        1,
                        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                        move |ctx| {
                            observed.fetch_add(1, Ordering::Relaxed);
                            ctx.get::<i64>(0)
                        },
                    )
                    .unwrap();
                    let (relations, sql) = if staged {
                        let rows = (0..rules)
                            .map(|c| format!("({c},{},{})", c % 2, c % 2))
                            .collect::<Vec<_>>()
                            .join(",");
                        let stages = vec![
                            format!("CREATE TEMP TABLE shared_candidates AS WITH families(family,kind) AS(VALUES(0,{}),(1,{})) SELECT f.family,n.file,n.id node,staged_producer_visit(n.id) observed FROM families f JOIN node n ON n.kind=f.kind", kinds[0], kinds[1]),
                            "CREATE UNIQUE INDEX shared_candidate_key ON shared_candidates(family,file,node)".into(),
                            format!("CREATE TEMP TABLE rule_demands AS WITH rules(clause,family,term) AS(VALUES {rows}) SELECT * FROM rules"),
                            "CREATE UNIQUE INDEX rule_demand_key ON rule_demands(family,clause)".into(),
                        ];
                        (json!({"spikeStages":stages,"spikeStageOutputs":["shared_candidates",null,"rule_demands",null],"spikeStagePurposes":["two closed producer families across all files","shared typed-key index","logical rule identities","consumer lookup index"]}), "SELECT d.clause,p.file,p.node FROM rule_demands d JOIN shared_candidates p ON p.family=d.family WHERE p.observed>=0 AND node_regex(d.term,p.file,p.node) ORDER BY d.clause,p.file,p.node".to_string())
                    } else {
                        let declarations = (0..rules).map(|c| format!("p{c} AS MATERIALIZED(SELECT file,id node,staged_producer_visit(id) observed FROM node WHERE kind={})", kinds[c % 2])).collect::<Vec<_>>().join(",");
                        let arms = (0..rules).map(|c| format!("SELECT {c} clause,file,node FROM p{c} WHERE observed>=0 AND node_regex({},file,node)", c % 2)).collect::<Vec<_>>().join(" UNION ALL ");
                        (
                            Value::Null,
                            format!("WITH {declarations} {arms} ORDER BY clause,file,node"),
                        )
                    };
                    let stages = spike_stages::run(db, &relations, &sql).unwrap();
                    let mut stmt = db.prepare(&sql).unwrap();
                    let trace = ExecutionTrace::start(db);
                    let rows = stmt
                        .query_map([], |r| {
                            Ok([
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ])
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    let statements = trace.count() + stages.executions;
                    drop(trace);
                    assert_eq!(rows, native);
                    assert_eq!(statements, if staged { 5 } else { 1 });
                    json!({"staged":staged,"files":files,"rules":rules,"rows":rows.len(),"producerVisits":visits.load(Ordering::Relaxed),"matchingStatements":statements,"vmSteps":stages.vm+i64::from(stmt.get_status(StatementStatus::VmStep)),"stageExecution":stages.profile(),"finalSql":sql,"scope":"pure Kind/Regex reference; shared physical families, separate logical rules; no full-catalog transfer claim"})
                });
                metrics.push(sample);
            }
            assert_eq!(metrics[1]["producerVisits"], (6 * files) as u64);
            assert_eq!(metrics[0]["producerVisits"], (3 * files * rules) as u64);
            samples.extend(metrics);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_STAGED_JOB_SPIKE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
    }
}

#[test]
fn structural_spike_prepared_family_queries_reset_between_jobs_without_reprepare() {
    use crate::code_rule_dsl::sql::timing::Started;
    let policies = [("identifier", "^(alpha|beta)$"), ("integer", "^[135]$")].map(|(kind, regex)| {
        CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"prepared-family","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"python","rule":{"kind":kind,"regex":regex}}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    });
    let grammars = policies
        .iter()
        .map(|p| grammar_matchers(p).first().unwrap())
        .collect::<Vec<_>>();
    let language = grammars[0].language;
    let kinds = [
        language.kinds().kind_id("identifier"),
        language.kinds().kind_id("integer"),
    ];
    let source = "alpha\nbeta\nother\n1\n2\n3\n";
    let mut evidence = vec![];
    for files in [1, 16] {
        let terminals = ["^(alpha|beta)$", "^[135]$"]
            .map(|r| Terminal::Regex(regex::Regex::new(r).unwrap()))
            .into();
        exec::with_terminals(
            (0..files)
                .map(|_| (tape(language, source), Default::default()))
                .collect(),
            terminals,
            |db, _, _| {
                // Empty schema creation is a recorded one-off; data population and
                // reset are charged on every job. No query is prepared in the loop.
                let schema_started = Started::now();
                let schema_trace = ExecutionTrace::start(db);
                db.execute_batch("CREATE TEMP TABLE shared_candidates(family INTEGER,file INTEGER,node INTEGER,PRIMARY KEY(family,file,node)) WITHOUT ROWID; CREATE TEMP TABLE rule_demands(clause INTEGER,family INTEGER,term INTEGER,PRIMARY KEY(family,clause)) WITHOUT ROWID;").unwrap();
                let schema_statements = schema_trace.count();
                drop(schema_trace);
                let schema = schema_started.elapsed();
                assert_eq!(schema_statements, 2);
                let insert_sql = format!(
                    "INSERT INTO shared_candidates WITH families(family,kind) AS(VALUES(0,{}),(1,{})) SELECT f.family,n.file,n.id FROM families f JOIN node n ON n.kind=f.kind",
                    kinds[0], kinds[1]
                );
                let prepare_started = Started::now();
                let mut reset_candidates = db.prepare("DELETE FROM shared_candidates").unwrap();
                let mut reset_rules = db.prepare("DELETE FROM rule_demands").unwrap();
                let mut insert_candidates = db.prepare(&insert_sql).unwrap();
                let mut insert_rules = db.prepare("INSERT INTO rule_demands SELECT value->>0,value->>1,value->>2 FROM json_each(?1)").unwrap();
                let mut select = db.prepare("SELECT d.clause,p.file,p.node FROM rule_demands d JOIN shared_candidates p ON p.family=d.family WHERE node_regex(d.term,p.file,p.node) ORDER BY d.clause,p.file,p.node").unwrap();
                let prepare = prepare_started.elapsed();
                let mut jobs = vec![];
                for rules in [2, 146, 32] {
                    let mut expected = vec![];
                    for clause in 0..rules {
                        for file in 0..files {
                            let tree = AstGrep::doc(tape(language, source));
                            for node in tree.root().dfs() {
                                let mut work = 10000;
                                if grammars[clause % 2]
                                    .match_root(node.clone(), &mut work)
                                    .unwrap()
                                    .is_some()
                                {
                                    expected.push([
                                        clause as i64,
                                        file as i64,
                                        node.node_id() as i64,
                                    ]);
                                }
                                assert_eq!(work, 10000);
                            }
                        }
                    }
                    expected.sort();
                    let input = serde_json::to_string(
                        &(0..rules).map(|c| [c, c % 2, c % 2]).collect::<Vec<_>>(),
                    )
                    .unwrap();
                    let trace = ExecutionTrace::start(db);
                    let started = Started::now();
                    reset_candidates.execute([]).unwrap();
                    reset_rules.execute([]).unwrap();
                    insert_candidates.execute([]).unwrap();
                    insert_rules.execute([input]).unwrap();
                    let actual = select
                        .query_map([], |r| {
                            Ok([
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ])
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    let execution = started.elapsed();
                    let count = trace.count();
                    drop(trace);
                    assert_eq!(
                        actual, expected,
                        "stale rows or changed native results after rules={rules}"
                    );
                    assert_eq!(count, 5);
                    let vm = [
                        &reset_candidates,
                        &reset_rules,
                        &insert_candidates,
                        &insert_rules,
                        &select,
                    ]
                    .iter()
                    .map(|s| i64::from(s.reset_status(StatementStatus::VmStep)))
                    .sum::<i64>();
                    jobs.push(json!({"rules":rules,"files":files,"matchingStatements":count,"vmSteps":vm,"executionCpuNs":execution.cpu,"executionWallNs":execution.wall,"rows":actual.len(),"preparationsDuringJob":0}));
                }
                evidence.push(json!({"jobs":jobs,"schemaStatementsOnce":schema_statements,"schemaCpuNsOnce":schema.cpu,"schemaWallNsOnce":schema.wall,"preparedStatementsOnce":5,"prepareCpuNsOnce":prepare.cpu,"prepareWallNsOnce":prepare.wall,"contract":"native pure-rule parity after repeated reset/populate/execute; schema and preparation separate, every per-job mutation counted"}));
            },
        );
    }
    if let Ok(path) = std::env::var("RERAIL_STAGED_JOB_SPIKE_EVIDENCE") {
        std::fs::write(
            format!("{path}.prepared.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}
