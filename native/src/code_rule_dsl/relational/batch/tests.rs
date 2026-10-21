use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl, eval,
    model::{ClauseKind, FileInput, Metrics, Snapshot},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

fn rule() -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"capacity","revision":1,"mode":"log","clauses":[{
        "id":"nodes","relational":{"partitionBy":"file","inputs":{"gate":{"evidence":{"as":"n","query":{"ast":{"language":"python","syntax":"valid","rule":{"kind":"module"}}},"project":{"path":"file.path"}}}},
        "syntax":{"name":"nodes","gate":"gate"},
        "stages":[{"name":"descendants","reachability":{"syntax":"nodes","roots":"SELECT __path,id FROM nodes WHERE kind='function_definition'"}}],
        "query":"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id FROM nodes n JOIN descendants d ON d.__path=n.__path AND d.node=n.id AND d.root=n.id WHERE n.kind='function_definition'","key":["id"]},
        "violation":{"code":"CAPACITY","message":"Capacity","anchor":"witness.span"}}]}).to_string(), "json".into(), include_str!("../../capabilities.json").into()).unwrap()
}
fn files(count: usize) -> Vec<FileInput> {
    (0..count)
        .map(|i| {
            FileInput::new(
                format!("f{i:03}.py"),
                "python".into(),
                Arc::new("def f(): return 1\n".into()),
                None,
                None,
                "test",
            )
            .unwrap()
        })
        .collect()
}
fn jobs<'a>(rule: &'a CompiledCodeRuleDsl, files: Vec<FileInput>) -> Vec<Job<'a>> {
    let code_rule = &rule.inner;
    let clause = code_rule.clauses.iter().find(|c| c.id == "nodes").unwrap();
    let ClauseKind::Relational(plan) = &clause.kind else {
        panic!("relational clause");
    };
    files
        .into_iter()
        .map(|file| {
            let snapshot = Snapshot {
                revision: 1,
                files: vec![file],
            };
            let rows = eval::relational_rows(
                code_rule,
                clause,
                plan,
                &snapshot,
                &mut Metrics::default(),
                eval::Engine::Native,
            )
            .unwrap();
            Job {
                code_rule,
                clause,
                rows,
                snapshot,
                facts: Vec::new(),
                budget: Context::work(&Arc::new(AtomicBool::new(false)), 1_000_000),
            }
        })
        .collect()
}
#[test]
fn resident_partitions_preserve_all_rows_and_original_relation_order() {
    let rule = rule();
    let mut inputs = files(6);
    inputs.swap(0, 4);
    inputs.reverse();
    let (control, _) = run(
        jobs(&rule, inputs.clone()),
        producer::MAX_RESIDENT_BYTES,
        usize::MAX,
    );
    let (partitioned, stats) = run(jobs(&rule, inputs.clone()), 256, usize::MAX);
    assert!(stats.partitions > 1);
    for ((expected, actual), input) in control.iter().zip(&partitioned).zip(inputs) {
        assert!(actual.failures.is_empty(), "{:?}", actual.failures);
        assert_eq!(actual.violations, expected.violations);
        assert_eq!(actual.violations.len(), 1);
        assert_eq!(actual.violations[0]["path"], input.path);
    }
}
#[test]
fn sqlite_storage_controls_admission_without_starting_every_session() {
    let rule = rule();
    let (result, stats) = run(jobs(&rule, files(12)), producer::MAX_RESIDENT_BYTES, 1);
    assert_eq!(stats.peak_sessions, 1);
    assert_eq!(stats.partitions, 12);
    assert!(stats.peak_sql_bytes > 1);
    assert!(
        result
            .iter()
            .all(|r| r.failures.is_empty() && r.violations.len() == 1)
    );
}
#[test]
fn an_oversized_relation_does_not_reject_fitting_peers() {
    let rule = rule();
    let mut inputs = files(5);
    inputs[2].source = Arc::new(
        (0..100)
            .map(|i| format!("def f{i}(): return {i}\n"))
            .collect(),
    );
    let (results, _) = run(jobs(&rule, inputs), 256, usize::MAX);
    for (index, result) in results.iter().enumerate() {
        if index == 2 {
            assert_eq!(result.failures.len(), 1);
            assert_eq!(result.failures[0]["message"], "producer capacity exceeded");
            assert!(result.violations.is_empty());
        } else {
            assert!(result.failures.is_empty());
            assert_eq!(result.violations.len(), 1);
        }
    }
}
#[test]
fn capacity_partitions_share_the_original_stage_allowance() {
    let rule = rule();
    let (results, stats) = run(jobs(&rule, files(40)), producer::MAX_RESIDENT_BYTES, 1);
    assert_eq!(stats.partitions, 40);
    assert!(
        results[..33]
            .iter()
            .all(|r| r.failures.is_empty() && r.violations.len() == 1)
    );
    assert!(results[33..].iter().all(|r| {
        r.violations.is_empty()
            && r.failures
                .iter()
                .any(|f| f["message"] == "producer stage limit")
    }));
}
