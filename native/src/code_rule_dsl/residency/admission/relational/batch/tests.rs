use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    model::{FileInput, Snapshot},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

fn rule() -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"capacity","revision":1,"mode":"log","clauses":[{
        "id":"nodes","relational":{"partitionBy":"file","inputs":{"gate":{"evidence":{"as":"n","query":{"ast":{"language":"python","syntax":"valid","rule":{"kind":"module"}}},"project":{"path":"file.path"}}}},
        "syntax":{"name":"nodes","gate":"gate"},
        "stages":[{"name":"descendants","reachability":{"syntax":"nodes","roots":"SELECT __path,id FROM nodes WHERE kind='function_definition'"}}],
        "query":"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id FROM nodes n JOIN descendants d ON d.__path=n.__path AND d.node=n.id AND d.root=n.id WHERE n.kind='function_definition'","key":["id"]},
        "violation":{"code":"CAPACITY","message":"Capacity","anchor":"witness.span"}}]}).to_string(), "json".into(), include_str!("../../../../capabilities.json").into()).unwrap()
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

    files
        .into_iter()
        .map(|file| {
            let snapshot = Snapshot {
                revision: 1,
                files: vec![file],
            };
            Job {
                code_rule,
                clause,
                inputs: Inputs {
                    gate: None,
                    file_local: false,
                    read_bytes: 0,
                    rows: None,
                    snapshot,
                    facts: Vec::new(),
                },
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

#[test]
fn pending_consumers_do_not_construct_any_inputs_and_failed_preparation_is_local() {
    use std::cell::Cell;
    let rule = rule();
    let pending = jobs(&rule, files(8));
    assert!(
        pending
            .iter()
            .all(|j| j.inputs.rows.is_none() && j.inputs.facts.is_empty())
    );
    let called = Cell::new(0);
    let (results, stats) = run_prepared(
        pending,
        producer::MAX_RESIDENT_BYTES,
        1,
        |inputs, _, _, phase| {
            assert_eq!(phase, Preparation::Body);
            assert!(inputs.rows.is_none());
            assert_eq!(inputs.snapshot.files.len(), 1);
            let n = called.get();
            called.set(n + 1);
            if inputs.snapshot.files[0].path == "f003.py" {
                return Err(super::super::invalid("preparation interrupted"));
            }
            Ok(())
        },
    );
    assert_eq!(called.get(), 8);
    assert_eq!(stats.peak_sessions, 1);
    assert_eq!(results[3].failures.len(), 1);
    assert!(results[3].violations.is_empty());
    for (n, result) in results.iter().enumerate().filter(|(n, _)| *n != 3) {
        assert!(result.failures.is_empty(), "consumer {n}");
        assert_eq!(result.violations.len(), 1);
    }
}

#[test]
fn projection_demand_includes_nested_predicates_and_wildcards_but_not_counted_row_values() {
    use crate::code_rule_dsl::model::ClauseKind;
    fn compile(query: &str, stages: serde_json::Value) -> Vec<String> {
        let document = json!({"format":"rerail.policy/v1","id":"columns","revision":1,"mode":"log","clauses":[{"id":"rows","relational":{"partitionBy":"file","inputs":{"facts":{"evidence":{"as":"f","query":{"ast":{"language":"python","rule":{"kind":"module"}}},"project":{"used":"file.path","unused":"file.name"}}}},"stages":stages,"query":query},"violation":{"code":"COLUMNS","message":"columns","anchor":"witness.span"}}]});
        let rule = CodeRuleDslRuntime::compile(
            document.to_string(),
            "json".into(),
            include_str!("../../../../capabilities.json").into(),
        )
        .unwrap();
        let ClauseKind::Relational(plan) = &rule.inner.clauses[0].kind else {
            panic!("relational");
        };
        plan.inputs[0]
            .1
            .project
            .iter()
            .map(|p| p.field.clone())
            .collect()
    }
    assert_eq!(
        compile(
            "SELECT __path path,__start start_byte,__end end_byte,used FROM facts",
            json!([])
        ),
        vec!["used"]
    );
    assert_eq!(
        compile(
            "SELECT f.__path path,f.__start start_byte,f.__end end_byte FROM facts f WHERE EXISTS(SELECT 1 FROM facts g WHERE g.unused=f.used)",
            json!([])
        ),
        vec!["unused", "used"]
    );
    assert!(compile("SELECT __path path,min(__start) start_byte,max(__end) end_byte,count(*) amount FROM facts GROUP BY __path",json!([])).is_empty());
    assert_eq!(
        compile(
            "SELECT __path path,__start start_byte,__end end_byte,used FROM all_fields",
            json!([{"name":"all_fields","query":"SELECT * FROM facts"}])
        ),
        vec!["unused", "used"]
    );
}

#[test]
fn narrowing_a_consumer_keeps_the_shared_ast_and_unmodified_schemas() {
    use crate::code_rule_dsl::model::{ClauseKind, Projection, ProjectionSource};
    let rule = rule();
    let ClauseKind::Relational(plan) = &rule.inner.clauses[0].kind else {
        panic!("relational");
    };
    let mut full = plan.inputs[0].1.clone();
    full.project = vec![
        Projection {
            field: "a".into(),
            source: ProjectionSource::Path,
            extract: None,
        },
        Projection {
            field: "b".into(),
            source: ProjectionSource::Name,
            extract: None,
        },
    ]
    .into();
    let mut narrow = full.clone();
    narrow.retain_projected_fields(Some(&["a".to_owned()].into_iter().collect()));
    assert!(
        Arc::ptr_eq(&full.ast, &narrow.ast),
        "column demand must never clone matcher trees"
    );
    assert_eq!(full.project.len(), 2);
    assert_eq!(narrow.project.len(), 1);
    let schema = Arc::clone(&full.project);
    full.retain_projected_fields(Some(
        &["a".to_owned(), "b".to_owned()].into_iter().collect(),
    ));
    assert!(Arc::ptr_eq(&full.project, &schema));
}

#[test]
fn sql_facts_keep_typed_values_and_only_the_capture_needed_for_the_anchor() {
    use crate::code_rule_dsl::{
        eval,
        model::{Anchor, ClauseKind, Metrics},
        violations::anchored_span,
    };
    let rule=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"input-payload","revision":1,"mode":"log","clauses":[{"id":"rows","relational":{"partitionBy":"file","inputs":{"facts":{"evidence":{"as":"f","query":{"ast":{"language":"python","rule":{"all":[{"kind":"function_definition"},{"has":{"field":"name","pattern":"$NAME"}},{"has":{"field":"parameters","pattern":"$PARAMS"}}]}}},"project":{"name":"capture.NAME"}}}},"query":"SELECT __path path,__start start_byte,__end end_byte,name FROM facts"},"violation":{"code":"INPUT","message":"unused input message","anchor":"witness.span"}}]}).to_string(),"json".into(),include_str!("../../../../capabilities.json").into()).unwrap();
    let code_rule = &rule.inner;
    let clause = &code_rule.clauses[0];
    let ClauseKind::Relational(plan) = &clause.kind else {
        panic!("relational");
    };
    let snapshot = Snapshot {
        revision: 1,
        files: files(1),
    };
    let rows =
        eval::relational_rows(code_rule, clause, plan, &snapshot, &mut Metrics::default()).unwrap();
    let witness = &rows[0].witnesses[0];
    assert_eq!(
        witness.captures.projection().unwrap().json(),
        json!({"name":"f"})
    );
    assert!(witness.captures.get("NAME").is_none());
    assert!(witness.captures.get("PARAMS").is_none());
    assert!(witness.message.is_none());
    assert!(witness.fix.is_none());
    let mut input = plan.inputs[0].1.clone();
    input.anchor = Anchor::Capture("NAME".into());
    let rows = eval::stateless_rows(
        code_rule,
        clause,
        (&input, eval::evidence::Side::Counted),
        &snapshot,
        &mut Metrics::default(),
    )
    .unwrap();
    let witness = &rows.witnesses[0];
    assert!(witness.captures.get("NAME").is_some());
    assert!(witness.captures.get("PARAMS").is_none());
    let (anchor, fallback) = anchored_span(&input.anchor, witness);
    assert_eq!((anchor.start_byte, anchor.end_byte), (4, 5));
    assert!(!fallback);
    assert!(anchored_span(&Anchor::Capture("ABSENT".into()), witness).1);
}
