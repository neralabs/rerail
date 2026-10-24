//! Same-plan physical comparison isolates total-predicate barriers from reached effects.
use super::super::{
    Slots,
    lower::Lower,
    publication_tests::{check_work, policy_utils},
};
use super::{Demand, Generation, Sql};
use crate::code_rule_dsl::{
    sql::{exec, profiling, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::{Value, json};
fn utils() -> Value {
    json!({"u":{"any":[{"kind":"identifier","regex":"^[au]$"},{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}})
}
#[test]
fn tracked_predicate_runs_preserve_work_unknown_withholding_and_reached_faults() {
    let rules = [
        json!({"kind":"identifier","all":[{"pattern":"$X","testUnknown":"^u$"},{"kind":"identifier"},{"regex":"^u$"},{"pattern":"$X"},{"matches":"u"}]}),
        json!({"kind":"identifier","all":[{"testUnknown":"^u$"},{"kind":"identifier"},{"regex":"^a$"},{"value":{"constant":true}},{"matches":"u"}]}),
        json!({"kind":"identifier","any":[{"pattern":"$X","testUnknown":"^u$"},{"kind":"identifier"},{"regex":"^never$"},{"value":{"constant":true}},{"matches":"u"}]}),
        json!({"kind":"identifier","any":[{"testUnknown":"^u$"},{"regex":"^never$"},{"kind":"number"},{"all":[{"value":{"constant":true}},{"matches":"u"}]}]}),
    ];
    let p = policy_utils(&rules, utils());
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 128, 1024, i64::MAX] {
        check_work(&p, &rules, &["a;u;1;[a,u];"], None, work);
    }
    for fault in ["a", "u"] {
        check_work(&p, &rules, &["a;u;1;[a,u];"], Some(fault), i64::MAX);
    }
}
fn measured(rule: Value, source: &str) -> Vec<Value> {
    let p = policy_utils(std::slice::from_ref(&rule), utils());
    check_work(&p, &[rule], &[source], None, i64::MAX);
    let grammars = grammar_matchers(&p);
    let grammar = &grammars[0];
    let mut lower = Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    exec::with_terminals(
        vec![(tape(grammar.language, source), Default::default())],
        lower.pure.program.terminals,
        |db, batch, _| {
            super::super::super::leaves::register(db, batch).unwrap();
            let mut samples = vec![];
            for disabled in [true, false] {
                db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
                    .unwrap();
                let mut sql = Sql {
                    definitions: vec![],
                    profile: vec![],
                    correlated: true,
                    generation: Generation {
                        peak_tracking: true,
                        disable_predicate_runs: disabled,
                        memo_clause: Some(0),
                        ..Generation::default()
                    },
                };
                let roots=Demand{name:sql.relation("measured root inputs","SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,1000000 AS work FROM node n".into(),true,"single-file inputs"),slots:Slots::new()};
                sql.generation.components.borrow_mut().roots = roots.name.clone();
                let output = sql.emit(&plan, &roots).unwrap();
                sql.publish_components();
                let query = format!(
                    "WITH RECURSIVE {} SELECT d.node,o.truth,o.work FROM {} d JOIN {} o ON o.k=d.k WHERE o.truth=1 ORDER BY d.node",
                    sql.definitions.join(",\n"),
                    roots.name,
                    output.name
                );
                let start = std::time::Instant::now();
                let mut stmt = db.prepare(&query).unwrap();
                let prepare = start.elapsed().as_nanos();
                let (rows, frames) = profiling::frame_counts(|| {
                    stmt.query_map([], |r| {
                        Ok([
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                        ])
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
                });
                samples.push(json!({"disabled":disabled,"output":rows,"sqlBytes":query.len(),"vmSteps":stmt.get_status(rusqlite::StatementStatus::VmStep),"nodeAdvances":frames[11],"prepareWallNs":prepare,"operators":sql.generation.operators.get()}));
            }
            samples
        },
    )
}
#[test]
fn tracked_predicate_runs_reduce_real_sql_and_vm_work_with_linear_root_growth() {
    let rules = [
        json!({"kind":"identifier","all":[{"regex":"."},{"not":{"regex":"^never$"}},{"kind":"identifier"},{"matches":"u"},{"regex":"."},{"not":{"kind":"number"}},{"kind":"identifier"}]}),
        json!({"kind":"identifier","any":[{"regex":"^never$"},{"kind":"number"},{"not":{"kind":"identifier"}},{"matches":"u"},{"regex":"."}]}),
    ];
    let mut evidence = vec![];
    for (style, rule) in rules.into_iter().enumerate() {
        let mut prior: Option<Vec<Value>> = None;
        for roots in [8, 32, 128] {
            let samples = measured(rule.clone(), &"a;u;b;".repeat(roots));
            assert_eq!(samples[0]["output"], samples[1]["output"]);
            assert!(samples[1]["sqlBytes"].as_u64() < samples[0]["sqlBytes"].as_u64());
            assert!(samples[1]["vmSteps"].as_u64() < samples[0]["vmSteps"].as_u64());
            if let Some(prior) = prior.as_ref() {
                for (before, after) in prior.iter().zip(&samples) {
                    assert_eq!(before["sqlBytes"], after["sqlBytes"]);
                    assert!(
                        after["vmSteps"].as_u64().unwrap()
                            <= 5 * before["vmSteps"].as_u64().unwrap()
                    );
                }
            }
            evidence.push(json!({"style":style,"roots":roots,"samples":samples}));
            prior = Some(samples);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_PREDICATE_FUSION_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&evidence).unwrap()).unwrap();
    }
}
